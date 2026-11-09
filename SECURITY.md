# Security policy

## Supported versions

Only the latest release receives fixes. strop ships one artifact: the static
`x86_64-unknown-linux-musl` binary installed by
[install.sh](https://strop.dev/install.sh) (Linux and WSL). Releases are built
locally from a clean tagged checkout that passed every blocking gate, and the
release catalog publishes each tarball's SHA-256; `install.sh` and
`strop update` refuse a tarball whose digest does not match.

## Reporting a vulnerability

Report privately through GitHub's
[private vulnerability reporting](https://github.com/stropdev/strop/security/advisories/new)
for this repository. Please include the strop version (`strop --version`), what
an attacker controls (a file you open, a remote host, a container, a language
server, a release download …) and a reproduction if you have one. Do not open a
public issue for an unfixed vulnerability.

You will get an acknowledgement within a week. Fixes ship in a new release and
the advisory is published with it, crediting the reporter unless they prefer
otherwise.

## Privacy

strop sends no telemetry and transfers no files, source text or terminal
content in the background. Network traffic happens only for operations you
start or configure: language servers, SSH, containers, Git remotes, and update
checks or downloads. Optional traces record content only under their
documented opt-in controls ([docs/terminal-and-scripting.md](docs/terminal-and-scripting.md));
inspect a trace before sharing it.
