"""Captured lock retirement before rmdir; never delete arrivals or ordinary entries."""


def validate_removal_directory(scope, descriptor, before):
    scope.revalidate_parents()
    scope.revalidate_lock()
    named = os.stat(scope.name, dir_fd=scope.parent, follow_symlinks=False)
    held = os.fstat(descriptor)
    current = metadata(held)
    if identity(named) != identity(held) or any(current[key] != before[key]
            for key in ('kind', 'identity', 'uid', 'gid', 'permissions', 'links')):
        raise Refusal('conflict', 'removal directory binding or permissions changed')


def directory_locks(scope, before):
    descriptor = scope.own(os.open(scope.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                   dir_fd=scope.parent))
    validate_removal_directory(scope, descriptor, before)
    captured = []
    total = 0
    with os.scandir(descriptor) as entries:
        for entry in entries:
            name = os.fsencode(entry.name)
            suffix = name[len(LOCK_PREFIX):] if name.startswith(LOCK_PREFIX) else b''
            if len(suffix) != 64 or any(byte not in b'0123456789abcdef' for byte in suffix):
                raise Refusal('unsupported', 'non-empty directory removal is not authorized')
            total += len(name)
            if len(captured) >= MAX_ENTRIES or total > 16 * 1024 * 1024:
                raise Refusal('unsupported', 'directory lock cleanup exceeds 100000 names or 16 MiB')
            info = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
            captured.append((name, lock_identity(info, True)))
    validate_removal_directory(scope, descriptor, before)
    return descriptor, captured


def prune_directory_locks(scope, before, state):
    descriptor, captured = directory_locks(scope, before)
    confirmed = 0
    try:
        for name, expected in captured:
            lock = os.open(name, os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=descriptor)
            try:
                if lock_identity(os.fstat(lock), True) != expected:
                    raise Refusal('conflict', 'captured operation lock changed identity')
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    raise Refusal('busy', 'another cooperating operation owns this name')
                current = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
                if lock_identity(current, True) != expected or lock_identity(os.fstat(lock), True) != expected:
                    raise Refusal('conflict', 'captured operation lock changed identity')
                validate_removal_directory(scope, descriptor, before)
                blocked = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM, signal.SIGHUP})
                try:
                    os.unlink(name, dir_fd=descriptor)
                    confirmed += 1
                    state['pruned_locks'] = confirmed
                    # Old waiters must revalidate the now-missing pathname.
                    # Close after unlink, before rmdir, to release our own .nfs entry.
                    closing, lock = lock, None
                    os.close(closing)
                finally:
                    signal.pthread_sigmask(signal.SIG_SETMASK, blocked)
            finally:
                if lock is not None:
                    os.close(lock)
        validate_removal_directory(scope, descriptor, before)
    except (OSError, Refusal) as error:
        raise Refusal(error_kind(error), 'directory removal not attempted; %d protocol lock removals confirmed; %s' % (confirmed, error))
    return descriptor
