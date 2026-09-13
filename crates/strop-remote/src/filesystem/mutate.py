"""Descriptor-relative mutations. Publication and cleanup retain explicit evidence."""

def attributes(descriptor):
    if not all(hasattr(os, name) for name in ('listxattr', 'getxattr', 'setxattr', 'removexattr')):
        raise Refusal('unsupported', 'bounded metadata preservation is unavailable')
    result = {}
    total = 0
    for name in os.listxattr(descriptor):
        value = os.getxattr(descriptor, name)
        total += len(os.fsencode(name)) + len(value)
        if len(value) > 65536 or total > 1048576:
            raise Refusal('unsupported', 'copy metadata exceeds its preservation bound')
        result[name] = value
    return result


def copy_contents(request, source, destination, descriptor):
    source_fd = None
    try:
        expected = request.get('before_source')
        source_info = None
        source_attributes = None
        if expected is not None:
            source_fd = os.open(source.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=source.parent)
            source_info = os.fstat(source_fd)
            plain = dict(expected, digest=None)
            if metadata(source_info) != plain:
                raise Refusal('conflict', 'copy source changed before opening')
            if source_info.st_mode & 0o6000:
                raise Refusal('unsupported', 'copy refuses set-id source permissions')
            source_attributes = attributes(source_fd)
        buffer_copy = request['buffer_copy']
        remaining = request.get('length', 0)
        if buffer_copy and not 0 <= remaining <= MAX_BODY:
            raise Refusal('unsupported', 'buffer copy exceeds the 256 MiB upload bound')
        if not buffer_copy and source_fd is None:
            raise Refusal('conflict', 'stored copy requires an existing regular file')
        digest = hashlib.sha256()
        while True:
            if buffer_copy:
                if remaining == 0:
                    break
                chunk = sys.stdin.buffer.read(min(CHUNK, remaining))
                if not chunk:
                    raise Refusal('protocol', 'buffer copy upload ended early')
                remaining -= len(chunk)
            else:
                chunk = os.read(source_fd, CHUNK)
                if not chunk:
                    break
            digest.update(chunk)
            view = memoryview(chunk)
            while view:
                count = os.write(descriptor, view)
                if count <= 0:
                    raise Refusal('io', 'copy stage write made no progress')
                view = view[count:]
        wanted = request.get('digest') if buffer_copy else expected['digest']
        if wanted is None or list(digest.digest()) != wanted:
            raise Refusal('conflict', 'copy content did not match the prepared snapshot')
        if source_fd is not None:
            if metadata(os.fstat(source_fd)) != metadata(source_info) or attributes(source_fd) != source_attributes:
                raise Refusal('conflict', 'source changed during copy')
            os.fchmod(descriptor, stat.S_IMODE(source_info.st_mode) & 0o777)
            for name in attributes(descriptor):
                if name not in source_attributes:
                    os.removexattr(descriptor, name)
            for name, value in source_attributes.items():
                os.setxattr(descriptor, name, value)
            if attributes(descriptor) != source_attributes or stat.S_IMODE(os.fstat(descriptor).st_mode) != stat.S_IMODE(source_info.st_mode):
                raise Refusal('unsupported', 'copy metadata could not be preserved')
            if not buffer_copy:
                os.utime(descriptor, ns=(source_info.st_atime_ns, source_info.st_mtime_ns))
        os.fsync(descriptor)
        source_path = source.path if source is not None else location(request.get('source'))
        if inspect(source_path, expected is not None and expected.get('digest') is not None) != expected:
            raise Refusal('conflict', 'source name changed during copy')
        if source is not None:
            source.revalidate_parents()
            source.revalidate_lock()
        destination.revalidate_parents()
        destination.revalidate_lock()
        return list(digest.digest())
    finally:
        if source_fd is not None:
            os.close(source_fd)


def apply(request, state):
    current = capability()
    expected_capability = request['capability']
    if any(current[key] != expected_capability[key] for key in ('principal', 'incarnation', 'no_replace')):
        raise Refusal('conflict', 'prepared host/principal capability changed')
    source_path = location(request.get('source'))
    destination_path = location(request.get('destination'))
    for logical, physical in ((request.get('logical_source'), source_path), (request.get('logical_destination'), destination_path)):
        if logical is not None and resolve(location(logical)) != physical:
            raise Refusal('conflict', 'a logical path alias changed since preparation')
    scopes = {}
    stage_name = None
    stage_fd = None
    stage_identity = None
    contents = None
    public_fd = None
    copied_digest = None
    try:
        for path in sorted(set(path for path in (source_path, destination_path) if path is not None)):
            if path == source_path and request.get('before_source') is None and request['buffer_copy']:
                continue
            scope = PathScope(path)
            scopes[path] = scope
            scope.open_parent()
            expected_parent = next((entry['value'] for entry in request['parents'] if bytes(entry['path']) == os.path.dirname(path)), None)
            if not same_object(expected_parent, metadata(os.fstat(scope.parent))):
                raise Refusal('conflict', 'parent identity changed or required creation did not commit')
            scope.acquire_lock()
        source = scopes.get(source_path)
        destination = scopes.get(destination_path)
        before = request.get('before_source')
        if source is not None:
            if inspect(source.path, before is not None and before.get('digest') is not None) != before:
                raise Refusal('conflict', 'source changed since preparation')
            if before is not None:
                exact_name(source.parent, source.name)
        if destination is not None and inspect(destination.path) is not None:
            raise Refusal('conflict', 'destination is occupied; overwrite is not authorized')
        if request.get('before_destination') is not None and not request.get('vacated'):
            raise Refusal('conflict', 'destination vacancy was not established by this plan')
        kind = request['kind']
        if kind == 'Rename':
            public_fd = pin_source_version(source, before)
        if kind == 'Copy':
            stage_name = STAGE_PREFIX + secrets.token_hex(16).encode('ascii')
            os.mkdir(stage_name, 0o700, dir_fd=destination.parent)
            stage_identity = identity(os.stat(stage_name, dir_fd=destination.parent, follow_symlinks=False))
            stage_fd = os.open(stage_name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=destination.parent)
            if identity(os.fstat(stage_fd)) != stage_identity:
                raise Refusal('conflict', 'copy stage changed while opening')
            if stat.S_IMODE(os.fstat(stage_fd).st_mode) != 0o700 or os.fstat(stage_fd).st_uid != os.geteuid():
                raise Refusal('permission', 'copy stage is not private and owned')
            contents = os.open(b'contents', os.O_CREAT | os.O_EXCL | os.O_RDWR | os.O_NOFOLLOW, 0o600, dir_fd=stage_fd)
            state['contents_identity'] = identity(os.fstat(contents))
            copied_digest = copy_contents(request, source, destination, contents)
        for scope in scopes.values():
            scope.revalidate_parents()
            scope.revalidate_lock()
        for logical, physical in ((request.get('logical_source'), source_path), (request.get('logical_destination'), destination_path)):
            if logical is not None and resolve(location(logical)) != physical:
                raise Refusal('conflict', 'a logical path alias changed before publication')
        if kind == 'Remove' and before['kind'] == 'Directory':
            prune_directory_locks(source, before, state)
        created = None
        # Signals may cancel before this point. A queued signal after publication
        # cannot turn a committed syscall into a failed-before-mutation receipt.
        blocked = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT, signal.SIGTERM, signal.SIGHUP})
        try:
            state['attempted'] = True
            if kind == 'CreateFile':
                public_fd = os.open(destination.name, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o666, dir_fd=destination.parent)
                state['published'] = True
                created = metadata(os.fstat(public_fd))
                state['publication'] = publication_witness(public_fd, list(hashlib.sha256(b'').digest()))
                os.fsync(public_fd)
            elif kind == 'CreateDirectory':
                os.mkdir(destination.name, 0o777, dir_fd=destination.parent)
                state['published'] = True
                public_fd = os.open(destination.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=destination.parent)
                created = metadata(os.fstat(public_fd))
                state['publication'] = publication_witness(public_fd)
                os.fsync(public_fd)
            elif kind == 'Rename':
                no_replace(source.parent, source.name, destination.parent, destination.name)
                state['published'] = True
                state['publication'] = publication_witness(public_fd, before.get('digest'))
            elif kind == 'Copy':
                state['publication'] = publication_witness(contents, copied_digest)
                os.link(b'contents', destination.name, src_dir_fd=stage_fd, dst_dir_fd=destination.parent, follow_symlinks=False)
                state['published'] = True
                public_fd = os.open(destination.name, os.O_PATH | os.O_NOFOLLOW, dir_fd=destination.parent)
                if identity(os.fstat(public_fd)) != state['contents_identity']:
                    raise Refusal('conflict', 'copy destination was replaced before acknowledgement')
                state['publication'] = publication_witness(public_fd, copied_digest)
            elif kind == 'Remove':
                if before['kind'] == 'Directory':
                    os.rmdir(source.name, dir_fd=source.parent)
                else:
                    os.unlink(source.name, dir_fd=source.parent)
                state['published'] = True
            else:
                raise Refusal('unsupported', 'operation has no protected remote executor')
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, blocked)
        for scope in scopes.values():
            os.fsync(scope.parent)
        source_after = inspect(source_path)
        destination_after = inspect(destination_path)
        valid = ((kind == 'Remove' and source_after is None) or
                 (kind == 'Rename' and source_after is None and same_object(before, destination_after)) or
                 (kind == 'Copy' and destination_after is not None and
                  destination_after['identity'] == metadata(os.fstat(contents))['identity']) or
                 (kind in ('CreateFile', 'CreateDirectory') and same_object(created, destination_after)))
        if not valid:
            raise Refusal('conflict', 'post-publication observations need verification')
        state['outcome'] = {'Committed': {'source_after': source_after, 'destination_after': destination_after, 'recovery': None, 'warnings': [], 'publication': state.get('publication')}}
    finally:
        cleanup = []
        if state.get('pruned_locks'):
            cleanup.append('%d captured protocol lock removals confirmed' % state['pruned_locks'])
        # Recover an owned public descriptor even when the publication syscall
        # succeeded but its caller observed an I/O failure before the flag update.
        if contents is not None and public_fd is None and state.get('publication') is not None:
            candidate = None
            try:
                candidate = os.open(destination.name, os.O_PATH | os.O_NOFOLLOW, dir_fd=destination.parent)
                if identity(os.fstat(candidate)) == state['contents_identity']:
                    public_fd = candidate
                    candidate = None
                    state['published'] = True
            except FileNotFoundError:
                pass
            except OSError as error:
                cleanup.append('publication descriptor unavailable: ' + str(error))
            finally:
                if candidate is not None:
                    os.close(candidate)
        active_error = sys.exc_info()[1]
        retain_stage = (contents is not None and public_fd is None and state.get('publication') is not None
                        and (state['published'] or (state.get('attempted') and isinstance(active_error, OSError) and error_kind(active_error) == 'io')))
        if retain_stage:
            try:
                state['publication'] = publication_witness(contents, copied_digest)
            except OSError as error:
                state['publication'] = None
                cleanup.append('publication witness unavailable: ' + str(error))
        if contents is not None:
            try:
                os.close(contents)
            except OSError as error:
                cleanup.append(str(error))
        if stage_fd is not None and not retain_stage:
            try:
                current = os.stat(stage_name, dir_fd=destination.parent, follow_symlinks=False)
                if identity(current) != stage_identity:
                    raise Refusal('conflict', 'copy stage changed identity; cleanup refused')
                try:
                    remaining = os.stat(b'contents', dir_fd=stage_fd, follow_symlinks=False)
                    if identity(remaining) != state.get('contents_identity'):
                        raise Refusal('conflict', 'copy contents changed identity; cleanup refused')
                    os.unlink(b'contents', dir_fd=stage_fd)
                except FileNotFoundError:
                    pass
                os.close(stage_fd)
                stage_fd = None
                os.rmdir(stage_name, dir_fd=destination.parent)
            except (OSError, Refusal) as error:
                cleanup.append('copy stage retained: ' + str(error))
            finally:
                if stage_fd is not None:
                    os.close(stage_fd)
        elif stage_name is not None:
            if stage_fd is not None:
                os.close(stage_fd)
            cleanup.append('private bookkeeping retained at ' + repr(os.path.join(destination.path.rsplit(b'/', 1)[0], stage_name)))
        if public_fd is not None:
            try:
                content = (state.get('publication') or {}).get('content')
                if content is None and kind == 'CreateFile':
                    content = list(hashlib.sha256(b'').digest())
                current_publication = publication_witness(public_fd, content)
                if state.get('publication') is not None and current_publication['identity'] != state['publication']['identity']:
                    raise Refusal('conflict', 'publication descriptor does not identify the intended object')
                state['publication'] = current_publication
                if state.get('outcome'):
                    current_destination = inspect(destination_path)
                    if not identifies_publication(current_publication, current_destination):
                        raise Refusal('conflict', 'destination changed before final acknowledgement')
                    state['outcome']['Committed']['destination_after'] = current_destination
                    state['outcome']['Committed']['publication'] = current_publication
            except (OSError, Refusal) as error:
                state['outcome'] = None
                state['ack_error'] = str(error)
                cleanup.append('publication verification: ' + str(error))
            finally:
                os.close(public_fd)
        for scope in scopes.values():
            try:
                scope.close()
            except OSError as error:
                cleanup.append(str(error))
        state['cleanup'] = cleanup
        if state.get('outcome'):
            state['outcome']['Committed']['warnings'].extend(cleanup)
    if state.get('ack_error'):
        raise Refusal('conflict', state['ack_error'])
    return state['outcome']


def synchronize_verified(request, destination):
    synced = set()
    for field in ('destination', 'source'):
        path = location(request.get(field))
        if path is None or (field == 'source' and request.get('before_source') is None and request.get('buffer_copy')):
            continue
        scope = PathScope(path)
        try:
            scope.open_parent()
            expected = next((parent['value'] for parent in request.get('parents', []) if bytes(parent['path']) == os.path.dirname(path)), None)
            if expected is not None and not same_object(expected, metadata(os.fstat(scope.parent))):
                raise Refusal('conflict', 'verification parent changed identity')
            if field == 'destination' and destination is not None:
                descriptor = scope.own(os.open(scope.name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=scope.parent))
                plain = dict(destination, digest=None)
                if metadata(os.fstat(descriptor)) != plain:
                    raise Refusal('conflict', 'verification target changed before synchronization')
                os.fsync(descriptor)
                if inspect(path) != plain:
                    raise Refusal('conflict', 'verification target changed during synchronization')
            parent_identity = identity(os.fstat(scope.parent))
            if parent_identity not in synced:
                os.fsync(scope.parent)
                synced.add(parent_identity)
            scope.revalidate_parents()
        finally:
            scope.close()


def verify(request):
    kind = request['kind']
    if kind not in ('Rename', 'CreateFile', 'CreateDirectory', 'Copy', 'Remove'):
        raise Refusal('unsupported', 'verification kind is unsupported')
    source = inspect(location(request.get('source')), bool(request.get('before_source') and request['before_source'].get('digest')))
    destination = inspect(location(request.get('destination')))
    if source == request.get('before_source') and destination == request.get('before_destination'):
        return 'Unchanged'
    publication = request.get('publication')
    if kind == 'Remove':
        committed = source is None
    else:
        expected_kind = (request.get('before_source') or {}).get('kind') if kind == 'Rename' else ('Directory' if kind == 'CreateDirectory' else 'File')
        committed = bool(destination and destination['kind'] == expected_kind and identifies_publication(publication, destination))
        if kind == 'Rename':
            committed = committed and source is None and same_object(request.get('before_source'), destination)
        if committed and publication.get('content') is not None:
            destination = inspect(location(request.get('destination')), True)
            committed = identifies_publication(publication, destination) and destination['digest'] == publication['content']
    if committed:
        synchronize_verified(request, destination)
        return {'Committed': {'source_after': source, 'destination_after': destination}}
    return {'Unknown': {'detail': 'current names do not prove the intended object/version; no automatic retry'}}
