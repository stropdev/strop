"""Shared descriptor-pinned path and cooperative lock boundary for fixed helpers."""
import errno
import fcntl
import hashlib
import os
import stat
import unicodedata

MAX_ENTRIES = 100_000
LOCK_PREFIX = b'.strop-lock-'


class Refusal(Exception):
    def __init__(self, kind, detail):
        self.kind = kind
        self.detail = detail
        super().__init__(detail)


class ResolveLink(Exception):
    def __init__(self, path):
        super().__init__(path)
        self.path = path


def reserved(component):
    normalized = unicodedata.normalize('NFKC', os.fsdecode(component)).casefold()
    return normalized.startswith(('.strop-lock-', '.strop-save-', '.strop-fs-'))


def exact_name(directory, name):
    with os.scandir(directory) as entries:
        for count, entry in enumerate(entries):
            if count >= MAX_ENTRIES:
                raise Refusal('unsupported', 'directory spelling check exceeds 100000 entries')
            stored = os.fsencode(entry.name)
            if stored == name:
                if reserved(stored):
                    raise Refusal('invalid_path', 'protected operation control paths are reserved')
                return
    raise Refusal('invalid_path', 'use exact stored path spelling; reopen through the directory browser')


def identity(info):
    return info.st_dev, info.st_ino


def lock_identity(info, empty=False):
    if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or
            info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600 or
            (empty and info.st_size != 0)):
        raise Refusal('permission', 'operation lock is not a private owned single-link file with the required contents')
    return identity(info)


class PathScope:
    MAX_LINK_HOPS = 40

    def __init__(self, path):
        self.path = path
        self.descriptors = []
        self.bindings = []
        self.parent = None
        self.lock = None
        self.parent_path = None

    def own(self, descriptor):
        self.descriptors.append(descriptor)
        return descriptor

    def open_parent(self):
        if not self.path.startswith(b'/') or b'\0' in self.path:
            raise Refusal('invalid_path', 'an absolute native path is required')
        components = [part for part in self.path.split(b'/') if part]
        if not components or any(part in (b'.', b'..') or reserved(part) for part in components):
            raise Refusal('invalid_path', 'canonical non-control path components are required')
        path = self.path
        for _ in range(self.MAX_LINK_HOPS):
            try:
                self.walk_parent(path)
                self.parent_path = os.path.realpath(os.path.dirname(path))
                self.revalidate_parents()
                return
            except ResolveLink as resolution:
                path = resolution.path
        raise Refusal('invalid_path', 'too many symbolic links in the path')

    def walk_parent(self, path):
        flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
        todo = [part for part in path.split(b'/') if part and part != b'.']
        parent = self.own(os.open(b'/', flags))
        done = []
        bindings = []
        while len(todo) > 1:
            component = todo[0]
            if component != b'..':
                if reserved(component):
                    raise Refusal('invalid_path', 'canonical non-control path components are required')
                exact_name(parent, component)
            try:
                child = self.own(os.open(component, flags, dir_fd=parent))
            except OSError as failure:
                if failure.errno not in (errno.ENOTDIR, errno.ELOOP):
                    raise
                info = os.stat(component, dir_fd=parent, follow_symlinks=False)
                if not stat.S_ISLNK(info.st_mode):
                    raise Refusal('invalid_path', 'a path component is not a directory')
                target = os.readlink(component, dir_fd=parent)
                if not target.startswith(b'/'):
                    target = b'/' + b'/'.join(done) + b'/' + target
                raise ResolveLink(target + b'/' + b'/'.join(todo[1:]))
            bindings.append((parent, component, child))
            done.append(component)
            todo = todo[1:]
            parent = child
        self.bindings = bindings
        self.parent = parent
        self.name = todo[0]
        if self.name == b'..' or reserved(self.name):
            raise Refusal('invalid_path', 'canonical non-control path components are required')

    def acquire_lock(self):
        self.lock_name = LOCK_PREFIX + hashlib.sha256(self.name).hexdigest().encode('ascii')
        self.lock = self.own(os.open(self.lock_name, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600, dir_fd=self.parent))
        lock_identity(os.fstat(self.lock))
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Refusal('busy', 'another cooperating operation holds this file name')
        self.revalidate_lock()

    def revalidate_parents(self):
        for parent, name, child in self.bindings:
            current = os.stat(name, dir_fd=parent, follow_symlinks=False)
            if not stat.S_ISDIR(current.st_mode) or identity(current) != identity(os.fstat(child)):
                raise Refusal('conflict', 'a parent directory changed identity')
        if self.parent_path is not None and os.path.realpath(os.path.dirname(self.path)) != self.parent_path:
            raise Refusal('conflict', 'a logical parent alias changed identity')

    def revalidate_lock(self):
        current = os.stat(self.lock_name, dir_fd=self.parent, follow_symlinks=False)
        if identity(current) != identity(os.fstat(self.lock)) or current.st_nlink != 1:
            raise Refusal('conflict', 'operation lock changed identity')

    def close(self):
        while self.descriptors:
            os.close(self.descriptors.pop())
