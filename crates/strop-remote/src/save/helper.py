"""Fixed owned save helper. Inputs are framed data, never executable text."""
import errno
import json
import secrets
import signal
import struct
import sys
import time

VERSION = 1
MAX_BYTES = 256 * 1024 * 1024
MAX_HEADER = 16 * 1024
MAX_ATTRIBUTES = 64 * 1024
CHUNK = 64 * 1024
STAGE_PREFIX = b'.strop-save-'





def terminated(signum, frame):
    raise Refusal('cancelled', 'termination observed')


for termination_signal in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
    signal.signal(termination_signal, terminated)




def file_info(info):
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns,
            info.st_ctime_ns, info.st_mode, info.st_uid, info.st_gid, info.st_nlink)


def attributes(descriptor):
    if not all(hasattr(os, name) for name in ('listxattr', 'getxattr', 'setxattr', 'removexattr')):
        raise Refusal('unsupported', 'extended-attribute preservation is unavailable')
    result = {}
    total = 0
    try:
        names = os.listxattr(descriptor)
    except OSError as error:
        if error.errno in (errno.ENOTSUP, errno.EOPNOTSUPP):
            return result
        raise
    for name in names:
        name = os.fsencode(name)
        value = os.getxattr(descriptor, name)
        total += len(name) + len(value)
        if total > MAX_ATTRIBUTES:
            raise Refusal('metadata', 'attribute inventory exceeds 64 KiB')
        result[name] = value
    return result


def attribute_digest(values):
    digest = hashlib.sha256()
    for name, value in sorted(values.items()):
        digest.update(struct.pack('>I', len(name)))
        digest.update(name)
        digest.update(struct.pack('>I', len(value)))
        digest.update(value)
    return list(digest.digest())


def validate_file(info):
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise Refusal('invalid_path', 'only regular single-link files may be edited')
    if info.st_uid != os.geteuid():
        raise Refusal('permission', 'remote saving requires a file owned by the authenticated user')
    if info.st_size > MAX_BYTES:
        raise Refusal('too_large', 'remote file exceeds 256 MiB')


def snapshot(descriptor):
    before = os.fstat(descriptor)
    validate_file(before)
    values = attributes(descriptor)
    digest = hashlib.sha256()
    os.lseek(descriptor, 0, os.SEEK_SET)
    length = 0
    while True:
        chunk = os.read(descriptor, CHUNK)
        if not chunk:
            break
        length += len(chunk)
        if length > MAX_BYTES:
            raise Refusal('too_large', 'remote file grew beyond 256 MiB')
        digest.update(chunk)
    after = os.fstat(descriptor)
    if file_info(before) != file_info(after) or values != attributes(descriptor):
        raise Refusal('conflict', 'file changed while its baseline was read')
    return {
        'device': after.st_dev, 'inode': after.st_ino, 'size': after.st_size,
        'mtime_ns': after.st_mtime_ns, 'ctime_ns': after.st_ctime_ns,
        'mode': stat.S_IMODE(after.st_mode), 'uid': after.st_uid, 'gid': after.st_gid,
        'content': list(digest.digest()), 'attributes': attribute_digest(values),
    }, values


def preserves(current, expected):
    return all(current[key] == expected[key] for key in
               ('device', 'mode', 'uid', 'gid', 'mtime_ns', 'attributes'))


class Transaction(PathScope):
    def __init__(self, path):
        super().__init__(path)
        self.target = None
        self.stage_directory = None
        self.stage_name = None
        self.stage_identity = None
        self.stage = None
        self.commit_started = False

    def open(self):
        self.open_parent()
        exact_name(self.parent, self.name)
        self.acquire_lock()
        self.target = self.own(os.open(self.name, os.O_RDWR | os.O_NOFOLLOW, dir_fd=self.parent))
        validate_file(os.fstat(self.target))
        self.revalidate()

    def revalidate(self, target=None):
        self.revalidate_parents()
        self.revalidate_lock()
        current = os.stat(self.name, dir_fd=self.parent, follow_symlinks=False)
        descriptor = self.target if target is None else target
        validate_file(current)
        if file_info(current) != file_info(os.fstat(descriptor)):
            raise Refusal('conflict', 'destination changed identity or metadata')

    def create_stage(self):
        self.stage_name = STAGE_PREFIX + secrets.token_hex(16).encode('ascii')
        os.mkdir(self.stage_name, 0o700, dir_fd=self.parent)
        self.stage_identity = identity(os.stat(self.stage_name, dir_fd=self.parent, follow_symlinks=False))
        self.stage_directory = self.own(os.open(self.stage_name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=self.parent))
        info = os.fstat(self.stage_directory)
        if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
            raise Refusal('permission', 'transaction directory is not private')
        self.stage = self.own(os.open(b'contents', os.O_CREAT | os.O_EXCL | os.O_RDWR | os.O_NOFOLLOW, 0o600, dir_fd=self.stage_directory))

    def restore_metadata(self, expected, values):
        descriptor = self.stage
        os.fchown(descriptor, expected['uid'], expected['gid'])
        os.fchmod(descriptor, expected['mode'])
        inherited = attributes(descriptor)
        for name in inherited:
            if name not in values:
                os.removexattr(descriptor, name)
        for name, value in values.items():
            if inherited.get(name) != value:
                os.setxattr(descriptor, name, value)
        os.utime(descriptor, ns=(os.fstat(self.target).st_atime_ns, expected['mtime_ns']))
        actual = os.fstat(descriptor)
        if (stat.S_IMODE(actual.st_mode) != expected['mode'] or actual.st_uid != expected['uid'] or
                actual.st_gid != expected['gid'] or actual.st_mtime_ns != expected['mtime_ns'] or
                attributes(descriptor) != values):
            raise Refusal('metadata', 'restored mode, ownership, timestamp or attributes differ')

    def cleanup(self):
        # Cleanup is bounded by the supervisor's TERM grace; SIGKILL may leave
        # this private directory, never a partially overwritten destination.
        signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM, signal.SIGINT, signal.SIGHUP})
        try:
            if self.stage_identity is not None:
                # NFS silly-rename: renaming or unlinking an open file
                # leaves a .nfsXXXX entry in its directory until the last
                # client handle closes, and rmdir then fails ENOTEMPTY on
                # a directory that is already logically empty. Close our
                # stage handle first so no linger is self-inflicted.
                if self.stage is not None:
                    os.close(self.stage)
                    self.descriptors.remove(self.stage)
                    self.stage = None
                if self.stage_directory is not None:
                    try:
                        os.unlink(b'contents', dir_fd=self.stage_directory)
                    except FileNotFoundError:
                        pass  # atomic rename already removed this owned entry
                current = os.stat(self.stage_name, dir_fd=self.parent, follow_symlinks=False)
                if identity(current) != self.stage_identity:
                    raise Refusal('conflict', 'transaction directory changed identity during cleanup')
                # The deferred .nfsXXXX removal can trail the close by a
                # server round-trip; ENOTEMPTY here is transient, so retry
                # briefly rather than report a committed save as failed.
                for attempt in range(20):
                    try:
                        os.rmdir(self.stage_name, dir_fd=self.parent)
                        break
                    except OSError as failure:
                        if failure.errno not in (errno.ENOTEMPTY, errno.EEXIST) or attempt == 19:
                            raise
                        time.sleep(0.05)
                os.fsync(self.parent)
        finally:
            for descriptor in reversed(self.descriptors):
                os.close(descriptor)


def request():
    header = sys.stdin.buffer.readline(MAX_HEADER + 1)
    if len(header) > MAX_HEADER or not header.endswith(b'\n'):
        raise Refusal('protocol', 'missing or oversized request header')
    value = json.loads(header)
    if value.get('version') != VERSION or value.get('operation') not in ('edit', 'save', 'verify'):
        raise Refusal('protocol', 'unsupported request version or operation')
    length = value.get('length')
    digest = value.get('digest')
    if type(length) is not int or not 0 <= length <= MAX_BYTES:
        raise Refusal('too_large', 'request length exceeds the supported bound')
    if not isinstance(digest, list) or len(digest) != 32 or any(type(byte) is not int or not 0 <= byte <= 255 for byte in digest):
        raise Refusal('protocol', 'invalid content digest')
    return value


def perform(transaction, requested):
    transaction.open()
    before, values = snapshot(transaction.target)
    transaction.revalidate()
    operation = requested['operation']
    length, wanted = requested['length'], requested['digest']
    if operation == 'edit':
        if before['size'] != length or before['content'] != wanted:
            raise Refusal('conflict', 'displayed snapshot differs from the remote file; refresh before editing')
        transaction.create_stage()
        transaction.restore_metadata(before, values)
        os.fsync(transaction.stage)
        os.fsync(transaction.stage_directory)
        os.fsync(transaction.parent)
        return {'status': 'ready', 'stamp': before}
    expected = requested.get('before')
    if not isinstance(expected, dict):
        raise Refusal('protocol', 'missing expected file version')
    if operation == 'verify':
        if before == expected:
            return {'status': 'unchanged', 'stamp': before}
        if before['size'] != length or before['content'] != wanted or not preserves(before, expected):
            raise Refusal('conflict', 'remote state matches neither the original nor the intended snapshot')
        os.fsync(transaction.target)
        os.fsync(transaction.parent)
        checked, _ = snapshot(transaction.target)
        transaction.revalidate()
        if checked != before:
            raise Refusal('conflict', 'remote file changed during verification')
        return {'status': 'written', 'stamp': checked}
    if before != expected:
        raise Refusal('conflict', 'remote content or metadata changed since editing was admitted')
    transaction.create_stage()
    digest = hashlib.sha256()
    remaining = length
    while remaining:
        chunk = sys.stdin.buffer.read(min(CHUNK, remaining))
        if not chunk:
            raise Refusal('protocol', 'upload ended before its declared length')
        digest.update(chunk)
        view = memoryview(chunk)
        while view:
            written = os.write(transaction.stage, view)
            if written <= 0:
                raise Refusal('io', 'stage write made no progress')
            view = view[written:]
        remaining -= len(chunk)
    if list(digest.digest()) != wanted:
        raise Refusal('protocol', 'upload content digest differs')
    transaction.restore_metadata(before, values)
    os.fsync(transaction.stage)
    os.fsync(transaction.stage_directory)
    current, _ = snapshot(transaction.target)
    transaction.revalidate()
    if current != before:
        raise Refusal('conflict', 'remote file changed during stage preparation')
    # Set before the syscall: even a signal between rename and its return must
    # not be misclassified as a proven precommit failure.
    transaction.commit_started = True
    os.replace(b'contents', transaction.name, src_dir_fd=transaction.stage_directory, dst_dir_fd=transaction.parent)
    os.fsync(transaction.stage_directory)
    os.fsync(transaction.parent)
    committed, _ = snapshot(transaction.stage)
    transaction.revalidate(transaction.stage)
    if committed['size'] != length or committed['content'] != wanted or not preserves(committed, before):
        raise Refusal('metadata', 'committed file differs from intended bytes or preserved metadata')
    return {'status': 'written', 'stamp': committed}


def main():
    transaction = None
    result = None
    error = None
    try:
        required = ('O_NOFOLLOW', 'O_DIRECTORY', 'scandir', 'fchown', 'fsync', 'replace')
        if not all(hasattr(os, name) for name in required) or not hasattr(signal, 'pthread_sigmask'):
            raise Refusal('unsupported', 'descriptor and cancellation capabilities are unavailable')
        requested = request()
        if len(sys.argv) != 2:
            raise Refusal('protocol', 'one native destination argument is required')
        transaction = Transaction(os.fsencode(sys.argv[1]))
        result = perform(transaction, requested)
    except Refusal as failure:
        error = failure
    except PermissionError as failure:
        error = Refusal('permission', str(failure))
    except OSError as failure:
        kind = 'invalid_path' if failure.errno in (errno.ELOOP, errno.ENOTDIR) else 'io'
        error = Refusal(kind, str(failure))
    except (ValueError, TypeError, KeyError, AttributeError) as failure:
        error = Refusal('protocol', str(failure))
    finally:
        if transaction is not None:
            try:
                transaction.cleanup()
            except (OSError, Refusal) as failure:
                detail = str(failure)
                error = Refusal('io', ((error.detail + '; ') if error else '') + 'cleanup: ' + detail)
    if error is not None:
        result = {'status': 'refused', 'kind': error.kind, 'detail': error.detail[:4096],
                  'unconfirmed': bool(transaction and transaction.commit_started)}
    sys.stdout.write(json.dumps({'version': VERSION, 'result': result}, separators=(',', ':')) + '\n')
    sys.stdout.flush()
    return 1 if error is not None else 0


sys.exit(main())
