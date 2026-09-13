"""Bounded native observations and capability identity for filesystem operations."""
import ctypes
import json
import platform
import secrets
import signal
import sys

CHUNK = 64 * 1024
MAX_HEADER = 64 * 1024
MAX_BODY = 256 * 1024 * 1024
STAGE_PREFIX = b'.strop-fs-'


def location(value):
    if value is None:
        return None
    if not isinstance(value, list) or len(value) > 16384:
        raise Refusal('invalid_path', 'native path exceeds its bound')
    path = bytes(value)
    if not path.startswith(b'/') or b'\0' in path:
        raise Refusal('invalid_path', 'an absolute native path is required')
    if any(reserved(part) for part in path.split(b'/')):
        raise Refusal('invalid_path', 'protected operation control paths are reserved')
    return path


def resolve(path):
    if path is None:
        return None
    name = os.path.basename(path)
    if name in (b'', b'.', b'..'):
        raise Refusal('invalid_path', 'operation requires a named non-root entry')
    return os.path.join(os.path.realpath(os.path.dirname(path)), name)


def timestamp(value):
    seconds, nanos = divmod(value, 1000000000)
    return {'seconds': seconds, 'nanos': nanos}


def metadata(info):
    kind = ('File' if stat.S_ISREG(info.st_mode) else
            'Directory' if stat.S_ISDIR(info.st_mode) else
            'SymbolicLink' if stat.S_ISLNK(info.st_mode) else 'Unknown')
    return {'kind': kind, 'identity': {'device': info.st_dev, 'inode': info.st_ino},
            'size': info.st_size, 'modified': timestamp(info.st_mtime_ns),
            'changed': timestamp(info.st_ctime_ns), 'permissions': stat.S_IMODE(info.st_mode),
            'uid': info.st_uid, 'gid': info.st_gid, 'links': info.st_nlink, 'digest': None}


def inspect(path, digest=False):
    if path is None:
        return None
    try:
        before = metadata(os.stat(path, follow_symlinks=False))
    except FileNotFoundError:
        return None
    if not digest or before['kind'] != 'File':
        return before
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        if metadata(os.fstat(descriptor)) != before:
            raise Refusal('conflict', 'source changed before inspection')
        value = hashlib.sha256()
        while True:
            chunk = os.read(descriptor, CHUNK)
            if not chunk:
                break
            value.update(chunk)
        if metadata(os.fstat(descriptor)) != before or inspect(path) != before:
            raise Refusal('conflict', 'source changed during inspection')
        before['digest'] = list(value.digest())
        return before
    finally:
        os.close(descriptor)


def same_object(left, right):
    return bool(left and right and left['identity'] == right['identity'] and left['kind'] == right['kind'])


def capability():
    if platform.system() != 'Linux':
        raise Refusal('unsupported', 'protected filesystem operations currently require Linux renameat2 and boot identity')
    library = ctypes.CDLL(None, use_errno=True)
    if not hasattr(library, 'renameat2'):
        raise Refusal('unsupported', 'native no-replace renameat2 is unavailable')
    with open('/proc/sys/kernel/random/boot_id', 'rb') as stream:
        boot = stream.read(128).strip().decode('ascii')
    if len(boot) != 36:
        raise Refusal('unsupported', 'native boot identity is unavailable')
    namespace = os.stat('/proc/self/ns/mnt')
    root = os.stat('/')
    return {'principal': os.geteuid(),
            'incarnation': '{}:{}:{}:{}:{}'.format(boot, namespace.st_dev, namespace.st_ino, root.st_dev, root.st_ino),
            'no_replace': True, 'trash': False, 'trash_root': None,
            'metadata_policy': 'create: 0666/0777 subject to umask; copy: permissions and bounded xattrs, stored-file mtime; owner is current user',
            'concurrency_policy': 'descriptor-pinned parents, final observations and cooperative name locks; no universal CAS against nonparticipants'}


def no_replace(source_parent, source_name, destination_parent, destination_name):
    library = ctypes.CDLL(None, use_errno=True)
    rename = library.renameat2
    rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    rename.restype = ctypes.c_int
    if rename(source_parent, source_name, destination_parent, destination_name, 1) != 0:
        code = ctypes.get_errno()
        if code in (errno.ENOSYS, errno.EINVAL, errno.EOPNOTSUPP, errno.EXDEV):
            raise Refusal('unsupported', 'native same-filesystem no-replace rename refused: ' + os.strerror(code))
        raise OSError(code, os.strerror(code))


def publication_witness(descriptor, content=None):
    observed = metadata(os.fstat(descriptor))
    return {'identity': observed['identity'], 'changed': observed['changed'], 'content': content}


def pin_source_version(scope, before):
    access = getattr(os, 'O_PATH', None)
    if access is None:
        access = getattr(os, 'O_EVTONLY', None)
    if access is None and sys.platform == 'darwin':
        access = 0x00008000  # Darwin O_EVTONLY: metadata without source read permission.
    if access is None:
        raise Refusal('unsupported', 'metadata-only source pinning is unavailable')
    descriptor = os.open(scope.name, access | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=scope.parent)
    try:
        if metadata(os.fstat(descriptor)) != dict(before, digest=None):
            raise Refusal('conflict', 'source changed before its publication identity was pinned')
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def identifies_publication(publication, observed):
    return bool(publication and observed and publication['identity'] == observed['identity']
                and publication.get('changed') is not None and publication['changed'] == observed['changed'])


def prepare(request):
    kind = request['kind']
    if kind in ('Trash', 'Restore'):
        raise Refusal('unsupported', 'remote native Trash/restore is unavailable; no unlink fallback')
    source = resolve(location(request.get('source')))
    destination = resolve(location(request.get('destination')))
    if source is not None and source == destination:
        raise Refusal('conflict', 'source and destination are the same resource')
    before = inspect(source, (kind == 'Copy' and not request['buffer_copy']) or request.get('expected_content') is not None)
    after = inspect(destination)
    if request.get('expected_content') is not None and (before is None or before['digest'] != request['expected_content']):
        raise Refusal('conflict', 'source content no longer matches the receipt intended bytes')
    if kind in ('CreateFile', 'CreateDirectory'):
        if source is not None or destination is None:
            raise Refusal('invalid_path', 'creation requires one destination')
    elif kind in ('Rename', 'Copy', 'Remove'):
        if source is None or (before is None and not (kind == 'Copy' and request['buffer_copy'])):
            raise Refusal('conflict', 'source no longer exists')
        if kind in ('Rename', 'Copy') and destination is None:
            raise Refusal('invalid_path', 'operation requires a destination')
        if kind == 'Remove' and destination is not None:
            raise Refusal('invalid_path', 'removal does not accept a destination')
        if before is not None:
            if before['kind'] not in ('File', 'Directory'):
                raise Refusal('unsupported', 'link and special-file mutations are unsupported')
            if before['kind'] == 'File' and before['links'] != 1 and kind != 'Copy':
                raise Refusal('unsupported', 'hard-linked file mutation requires explicit alias handling')
            if kind == 'Copy' and before['kind'] != 'File':
                raise Refusal('unsupported', 'copy supports regular files, not recursive directories')
            if before['kind'] == 'Directory' and destination and os.path.commonpath([source, destination]) == source:
                raise Refusal('invalid_path', 'a directory cannot move into itself')
    else:
        raise Refusal('protocol', 'unknown filesystem operation')
    if after is not None and not request['allow_occupied']:
        raise Refusal('conflict', 'destination is occupied; overwrite is not authorized')
    parents = []
    for path in (source, destination):
        if path is None:
            continue
        if path == source and before is None and request['buffer_copy']:
            continue
        parent = os.path.dirname(path)
        if any(entry['path'] == list(parent) for entry in parents):
            continue
        value = inspect(parent)
        if value and value['kind'] != 'Directory':
            raise Refusal('invalid_path', 'parent is not a directory')
        parents.append({'path': list(parent), 'value': value})
    return {'source': None if source is None else {'path': list(source), 'value': before},
            'destination': None if destination is None else {'path': list(destination), 'value': after},
            'parents': parents, 'capability': capability()}
