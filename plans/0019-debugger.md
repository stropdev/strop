# 0019 — Debugger research (superseded by 0060)

The canonical debugger release and architecture contract is now
[0060 — debugger workflow and architecture](0060-debugger-workflow-and-architecture.md).
Follow its **DBG01–DBG16** ledger, journeys, keybindings, module split, protocol
lifetimes, SSH/container capability envelope and real-adapter evidence gates.
Its entry sequence is [0056 architecture](0056-architecture-prerequisites.md),
[0057 whole-core verification](0057-core-verification-and-assurance.md),
[0058 native worker](0058-unified-native-worker.md), then
[0059 completion](0059-nonblocking-code-completion.md). Debugger consumes their
requalified native contracts, not generic repairs, a new helper or deferred proofs.

The original two adapter families and `Space D` namespace remain useful decisions.
The earlier claims that DAP is literally LSP, a debug-console REPL needs terminal
emulation, or `preLaunchTask` can be ignored are replaced by the researched 0060
contract. The old restricted first-version exclusions are not release authority.

GUI parity is [0061](0061-gui-windows-and-wsl.md); installation/distribution is
[0062](0062-distribution-and-wsl-onboarding.md). Debugging remains a shared engine
feature, not a GUI-only subsystem.
