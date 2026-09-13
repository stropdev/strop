"""One fixed framed request; no request field is executable source."""

def error_kind(error):
    if isinstance(error, Refusal):
        return error.kind
    if isinstance(error, PermissionError) or (isinstance(error, OSError) and error.errno == errno.EROFS):
        return 'permission'
    if isinstance(error, OSError):
        if error.errno in (errno.EEXIST, errno.ENOTEMPTY, errno.ENOENT):
            return 'conflict'
        if error.errno in (errno.ENOTDIR, errno.ELOOP, errno.EINVAL):
            return 'invalid_path'
        if error.errno in (errno.ENOSYS, errno.EOPNOTSUPP):
            return 'unsupported'
        return 'io'
    return 'protocol'

def terminated(signum, frame):
    raise Refusal('cancelled', 'termination observed')


def main():
    state = {'published': False, 'attempted': False, 'publication': None, 'outcome': None, 'cleanup': []}
    request = {}
    try:
        for number in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            signal.signal(number, terminated)
        header = sys.stdin.buffer.readline(MAX_HEADER + 1)
        if len(header) > MAX_HEADER or not header.endswith(b'\n'):
            raise Refusal('protocol', 'filesystem request header exceeds its bound or is incomplete')
        request = json.loads(header)
        if request.get('version') != 1:
            raise Refusal('protocol', 'filesystem protocol version differs')
        action = request.get('action')
        if action == 'prepare':
            value = prepare(request)
        elif action == 'apply':
            value = apply(request, state)
        elif action == 'verify':
            current = capability()
            if any(current[key] != request['capability'][key] for key in ('principal', 'incarnation')):
                raise Refusal('conflict', 'prepared host/principal capability changed')
            value = verify(request)
        else:
            raise Refusal('protocol', 'unknown filesystem request')
        reply = {'version': 1, 'value': value}
    except (Refusal, OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        kind = error_kind(error)
        detail = str(error)
        if state['cleanup']:
            detail += '; cleanup: ' + '; '.join(state['cleanup'])
        uncertain = state['published'] or (state['attempted'] and isinstance(error, OSError) and kind == 'io')
        if state['outcome'] is not None:
            reply = {'version': 1, 'value': state['outcome']}
        else:
            observed = None
            if uncertain:
                try:
                    observed = inspect(location(request.get('destination')))
                except (OSError, Refusal) as observation_error:
                    detail += '; post-publication observation: ' + str(observation_error)
            reply = {'version': 1, 'error': {'kind': kind, 'detail': detail[:4096], 'unconfirmed': uncertain,
                     'observed_destination': observed, 'publication': state['publication'] if uncertain else None}}
    sys.stdout.write(json.dumps(reply, separators=(',', ':')) + '\n')
    sys.stdout.flush()


main()
