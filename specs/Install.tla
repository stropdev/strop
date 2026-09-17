---- MODULE Install ----
(***************************************************************************)
(* TUI install/update publication safety (0057 VF16) over the AR11/AR12    *)
(* contract as implemented in install.sh and                               *)
(* crates/strop/src/update.rs, resolving through the generated release     *)
(* catalog (.github/scripts/release-catalog.py).                           *)
(*                                                                       *)
(*   TRANSACTION  one staged, verified, atomic transaction: read the      *)
(*                private installation receipt (.strop-install.json),     *)
(*                resolve the catalog (single source of truth for         *)
(*                version/tag/digest facts), download, verify sha256      *)
(*                against the catalog digest, stage in the destination    *)
(*                filesystem, atomically rename over the target, then     *)
(*                refresh the receipt with previous-version               *)
(*                carry-forward. Interruption before the rename           *)
(*                preserves any old installation; a digest mismatch       *)
(*                aborts before anything is staged over the target.       *)
(*                                                                       *)
(*   IDENTITY     channel identity comes from the receipt ALONE — never   *)
(*                path substrings. Brew/cargo/mise/unknown/missing/       *)
(*                malformed receipts are honest refusals with manager     *)
(*                routing, never overwrite authorization.                 *)
(*                                                                       *)
(*   CATALOG      the catalog's version/digest facts are the verified     *)
(*                truth: a not-newer catalog is a truthful no-op, never   *)
(*                a publication; the published version is exactly the     *)
(*                resolved catalog version.                               *)
(*                                                                       *)
(* THE invariants (VF16's named safety properties):                       *)
(*                                                                       *)
(*   TypeOK                  every variable stays in its declared finite  *)
(*                           domain                                       *)
(*   InstalledComplete       the target always holds a complete known     *)
(*                           artifact or nothing — never staged-partial   *)
(*                           bytes (atomic rename, or no rename)          *)
(*   VerifiedBeforeActivation                                           *)
(*                           a version beyond the initially installed     *)
(*                           one is present only when its activation      *)
(*                           passed digest verification                   *)
(*   ChannelGate             an activation happened only under a tarball  *)
(*                           receipt — foreign/missing identity was       *)
(*                           never overwritten                            *)
(*   NoClobberOnFailure      an aborted/refused/interrupted transaction   *)
(*                           left the installed artifact untouched        *)
(*   ReceiptNeverAhead       the receipt only ever names a version that   *)
(*                           was actually installed                       *)
(*   StaleNeverPublishes     an activation always moved to a version      *)
(*                           newer than its base                          *)
(*                                                                       *)
(* Deliberately faulty variants (configs flip MUTATION):                  *)
(*   MUTATION = 1  publish (rename) before digest verification;           *)
(*                 must die by VerifiedBeforeActivation                   *)
(*   MUTATION = 2  non-atomic in-place publication with an interruption   *)
(*                 window mid-write; must die by InstalledComplete and    *)
(*                 NoClobberOnFailure                                     *)
(*   MUTATION = 3  channel inferred from path substrings, overwriting     *)
(*                 manager-owned installs; must die by ChannelGate        *)
(*   MUTATION = 4  the receipt is refreshed even when the transaction     *)
(*                 failed; must die by ReceiptNeverAhead                  *)
(*   MUTATION = 5  a stale (not-newer) catalog publishes anyway;          *)
(*                 must die by StaleNeverPublishes                        *)
(*                                                                       *)
(* Not modeled: byte content/digest arithmetic (the verified/unverified   *)
(* distinction is the boundary), TLS/signature/provenance trust           *)
(* differences (named in AR12 as distinct, out of scope here), progress   *)
(* rendering, the promotion ledger (a release-workflow fact, natively     *)
(* tested), and liveness (a stuck curl is a progress matter). The         *)
(* receipt write failure AFTER a successful rename is the honest lag      *)
(* path (update.rs warns; the receipt keeps its previous identity).       *)
(***************************************************************************)
EXTENDS Integers, FiniteSets, TLC

CONSTANTS MUTATION     \* 0 = honest; faulty variants above

\* Versions: "none" < "v1" < "v2" for the newer-than decision.
Newer(v, base) ==
    \/ base = "none"
    \/ base = "v1" /\ v = "v2"

VARIABLES installed,   \* the target artifact: none|v1|v2|partial
          initInst,    \* what was installed when the behavior began
          everInstalled, \* every version actually installed so far
          receiptChannel, \* absent|malformed|tarball|brew|cargo|mise|unknown
          receiptVersion, \* "" when absent/malformed
          receiptPrevious, \* carry-forward: "" or a version
          phase,       \* idle|resolved|fetching|downloaded|verified|staged|
                     \* midwrite|published|aborted|refused
          txChannel,   \* the receipt channel this transaction admitted
          txStartInstall, \* installed when the transaction began
          catVer,      \* the resolved catalog version for this transaction
          tampered,    \* the downloaded bytes fail the catalog digest
          stagedHere,  \* a staged temp artifact exists beside the target
          actVer,      \* activation log: version ("" = never activated)
          actVerified, \* activation log: digest verification passed
          actChannel,  \* activation log: receipt channel that authorized it
          actBase,     \* activation log: the version it superseded
          publishes,   \* successful publications (witness accounting)
          digestRefusals, interrupts, managerRefusals, unknownRefusals,
          staleNoops,  \* truthful "already current" outcomes
          receiptLags  \* publish ok, receipt refresh failed (honest lag)

TypeOK ==
    /\ installed \in {"none", "v1", "v2", "partial"}
    /\ initInst \in {"none", "v1"}
    /\ everInstalled \in SUBSET {"v1", "v2"}
    /\ receiptChannel \in {"absent", "malformed", "tarball", "brew",
                           "cargo", "mise", "unknown"}
    /\ receiptVersion \in {"", "v1", "v2"}
    /\ receiptPrevious \in {"", "v1", "v2"}
    /\ phase \in {"idle", "resolved", "fetching", "downloaded", "verified",
                  "staged", "midwrite", "published", "aborted", "refused"}
    /\ txChannel \in {"", "tarball", "brew", "cargo", "mise", "unknown"}
    /\ txStartInstall \in {"", "none", "v1", "v2"}
    /\ catVer \in {"", "v1", "v2"}
    /\ tampered \in BOOLEAN
    /\ stagedHere \in BOOLEAN
    /\ actVer \in {"", "v1", "v2"}
    /\ actVerified \in BOOLEAN
    /\ actChannel \in {"", "tarball", "brew", "cargo", "mise", "unknown"}
    /\ actBase \in {"", "none", "v1", "v2"}
    /\ publishes \in 0..2
    /\ digestRefusals \in 0..2
    /\ interrupts \in 0..2
    /\ managerRefusals \in 0..2
    /\ unknownRefusals \in 0..2
    /\ staleNoops \in 0..2
    /\ receiptLags \in 0..2

Init ==
    \* A managed/pre-receipt install (v1) or no install at all; the
    \* receipt is whatever the disk holds — TLC explores every identity.
    /\ installed \in {"none", "v1"}
    /\ initInst = installed
    /\ everInstalled = IF installed = "none" THEN {} ELSE {"v1"}
    /\ receiptChannel \in {"absent", "malformed", "tarball", "brew",
                           "cargo", "mise", "unknown"}
    \* A receipt exists only beside an installation it describes.
    /\ (receiptChannel \in {"tarball", "brew", "cargo", "mise",
                            "unknown"}) => installed = "v1"
    /\ receiptVersion = IF receiptChannel \in {"tarball", "brew", "cargo",
                                               "mise", "unknown"}
                        THEN "v1" ELSE ""
    /\ receiptPrevious = ""
    /\ phase = "idle"
    /\ txChannel = ""
    /\ txStartInstall = ""
    /\ catVer = ""
    /\ tampered = FALSE
    /\ stagedHere = FALSE
    /\ actVer = ""
    /\ actVerified = FALSE
    /\ actChannel = ""
    /\ actBase = ""
    /\ publishes = 0
    /\ digestRefusals = 0
    /\ interrupts = 0
    /\ managerRefusals = 0
    /\ unknownRefusals = 0
    /\ staleNoops = 0
    /\ receiptLags = 0

\* Read the private receipt. Missing/malformed is an honest unknown;
\* manager channels route to their manager; tarball proceeds. MUTATION 3
\* infers the channel from path substrings and proceeds regardless.
BeginTx ==
    /\ phase = "idle"
    /\ publishes < 2
    /\ \/ \* Fresh bootstrap (install.sh on a clean host): no old binary,
          \* nothing to preserve; the install ESTABLISHES the tarball
          \* receipt (previous = none).
          /\ installed = "none"
          /\ txChannel' = "tarball"
          /\ txStartInstall' = installed
          /\ catVer' \in {"v1", "v2"}
          /\ phase' = "resolved"
          /\ UNCHANGED <<unknownRefusals, managerRefusals>>
       \/ \* Missing/malformed receipt beside an installation: an honest
          \* unknown, never a guess.
          /\ installed # "none"
          /\ receiptChannel \in {"absent", "malformed"}
          /\ unknownRefusals' = IF unknownRefusals < 2
                                THEN unknownRefusals + 1 ELSE unknownRefusals
          /\ phase' = "refused"
          /\ txStartInstall' = installed
          /\ UNCHANGED <<txChannel, catVer, managerRefusals>>
       \/ \* Manager-owned: route to the manager, never overwrite.
          /\ receiptChannel \in {"brew", "cargo", "mise", "unknown"}
          /\ MUTATION # 3
          /\ managerRefusals' = IF managerRefusals < 2
                                THEN managerRefusals + 1 ELSE managerRefusals
          /\ phase' = "refused"
          /\ txStartInstall' = installed
          /\ UNCHANGED <<txChannel, catVer, unknownRefusals>>
       \/ \* Tarball receipt (or MUTATION 3's path-substring inference,
          \* which proceeds regardless): resolve the release catalog.
          /\ (receiptChannel = "tarball" \/ MUTATION = 3)
          /\ receiptChannel \in {"tarball", "brew", "cargo", "mise",
                                 "unknown"}
          /\ txChannel' = receiptChannel
          /\ txStartInstall' = installed
          /\ catVer' \in {"v1", "v2"}
          /\ phase' = "resolved"
          /\ UNCHANGED <<unknownRefusals, managerRefusals>>
    /\ UNCHANGED <<installed, initInst, everInstalled, receiptChannel,
                   receiptVersion, receiptPrevious, tampered, stagedHere,
                   actVer, actVerified, actChannel, actBase, publishes,
                   digestRefusals, interrupts, staleNoops, receiptLags>>

\* The catalog answered. A not-newer catalog is a truthful no-op (never a
\* publication); a newer one proceeds to the download. MUTATION 5
\* publishes from a stale catalog anyway.
ResolveDecision ==
    /\ phase = "resolved"
    /\ IF Newer(catVer, installed) \/ MUTATION = 5
       THEN /\ phase' = "fetching"
            /\ UNCHANGED staleNoops
       ELSE /\ staleNoops' = IF staleNoops < 2 THEN staleNoops + 1
                             ELSE staleNoops
            /\ phase' = "idle"
    /\ UNCHANGED <<installed, initInst, everInstalled, receiptChannel,
                   receiptVersion, receiptPrevious, txChannel,
                   txStartInstall, catVer, tampered, stagedHere, actVer,
                   actVerified, actChannel, actBase, publishes,
                   digestRefusals, interrupts, managerRefusals,
                   unknownRefusals, receiptLags>>

\* Download the catalog's artifact. The network is untrusted: the bytes
\* may fail the digest.
Download ==
    /\ phase = "fetching"
    /\ tampered' \in BOOLEAN
    /\ phase' = "downloaded"
    /\ UNCHANGED <<installed, initInst, everInstalled, receiptChannel,
                   receiptVersion, receiptPrevious, txChannel,
                   txStartInstall, catVer, stagedHere, actVer, actVerified,
                   actChannel, actBase, publishes, digestRefusals,
                   interrupts, managerRefusals, unknownRefusals, staleNoops,
                   receiptLags>>

\* Verify the downloaded bytes against the catalog digest. A mismatch
\* aborts before anything reaches the target.
Verify ==
    /\ phase = "downloaded"
    /\ IF tampered
       THEN /\ digestRefusals' = IF digestRefusals < 2
                                 THEN digestRefusals + 1 ELSE digestRefusals
            /\ phase' = "aborted"
       ELSE /\ phase' = "verified"
            /\ UNCHANGED digestRefusals
    /\ UNCHANGED <<installed, initInst, everInstalled, receiptChannel,
                   receiptVersion, receiptPrevious, txChannel,
                   txStartInstall, catVer, tampered, stagedHere, actVer,
                   actVerified, actChannel, actBase, publishes, interrupts,
                   managerRefusals, unknownRefusals, staleNoops,
                   receiptLags>>

\* Stage in the destination filesystem (private temp next to the target).
Stage ==
    /\ phase = "verified"
    /\ stagedHere' = TRUE
    /\ phase' = "staged"
    /\ UNCHANGED <<installed, initInst, everInstalled, receiptChannel,
                   receiptVersion, receiptPrevious, txChannel,
                   txStartInstall, catVer, tampered, actVer, actVerified,
                   actChannel, actBase, publishes, digestRefusals,
                   interrupts, managerRefusals, unknownRefusals, staleNoops,
                   receiptLags>>

\* THE linearization point: the atomic rename over the target. Only a
\* staged, verified artifact reaches it.
PublishAtomic ==
    /\ phase = "staged"
    /\ MUTATION # 2
    /\ installed' = catVer
    /\ everInstalled' = everInstalled \cup {catVer}
    /\ actVer' = catVer
    /\ actVerified' = TRUE
    /\ actChannel' = txChannel
    /\ actBase' = txStartInstall
    /\ stagedHere' = FALSE
    /\ phase' = "published"
    /\ publishes' = publishes + 1
    /\ UNCHANGED <<initInst, receiptChannel, receiptVersion, receiptPrevious,
                   txChannel, txStartInstall, catVer, tampered,
                   digestRefusals, interrupts, managerRefusals,
                   unknownRefusals, staleNoops, receiptLags>>

\* MUTATION 1: rename BEFORE digest verification.
PublishEarly ==
    /\ MUTATION = 1
    /\ phase = "downloaded"
    /\ installed' = catVer
    /\ everInstalled' = everInstalled \cup {catVer}
    /\ actVer' = catVer
    /\ actVerified' = FALSE
    /\ actChannel' = txChannel
    /\ actBase' = txStartInstall
    /\ phase' = "published"
    /\ publishes' = publishes + 1
    /\ UNCHANGED <<initInst, receiptChannel, receiptVersion, receiptPrevious,
                   txChannel, txStartInstall, catVer, tampered, stagedHere,
                   digestRefusals, interrupts, managerRefusals,
                   unknownRefusals, staleNoops, receiptLags>>

\* MUTATION 2: publish by overwriting the target in place — a window
\* where the target holds staged-partial bytes.
PublishInPlaceStart ==
    /\ MUTATION = 2
    /\ phase = "staged"
    /\ installed' = "partial"
    /\ phase' = "midwrite"
    /\ UNCHANGED <<initInst, everInstalled, receiptChannel, receiptVersion,
                   receiptPrevious, txChannel, txStartInstall, catVer,
                   tampered, stagedHere, actVer, actVerified, actChannel,
                   actBase, publishes, digestRefusals, interrupts,
                   managerRefusals, unknownRefusals, staleNoops,
                   receiptLags>>

FinishInPlace ==
    /\ MUTATION = 2
    /\ phase = "midwrite"
    /\ installed' = catVer
    /\ everInstalled' = everInstalled \cup {catVer}
    /\ actVer' = catVer
    /\ actVerified' = TRUE
    /\ actChannel' = txChannel
    /\ actBase' = txStartInstall
    /\ stagedHere' = FALSE
    /\ phase' = "published"
    /\ publishes' = publishes + 1
    /\ UNCHANGED <<initInst, receiptChannel, receiptVersion, receiptPrevious,
                   txChannel, txStartInstall, catVer, tampered,
                   digestRefusals, interrupts, managerRefusals,
                   unknownRefusals, staleNoops, receiptLags>>

\* Refresh the receipt atomically beside the binary: same channel, new
\* version, previous-version carry-forward. The write may fail AFTER the
\* rename — the install stands, the receipt honestly keeps its previous
\* identity (update.rs warns). MUTATION 4 refreshes the receipt for a
\* FAILED transaction instead.
ReceiptRefresh ==
    /\ phase = "published" \/ (MUTATION = 4 /\ phase = "aborted")
    /\ IF MUTATION = 4 /\ phase = "aborted"
       THEN /\ receiptVersion' = catVer
            /\ UNCHANGED <<receiptChannel, receiptPrevious, receiptLags>>
       ELSE \/ /\ receiptChannel' = "tarball"
               /\ receiptVersion' = catVer
               /\ receiptPrevious' = IF txStartInstall = "none"
                                     THEN "" ELSE txStartInstall
               /\ UNCHANGED receiptLags
            \/ /\ receiptLags' = IF receiptLags < 2 THEN receiptLags + 1
                                 ELSE receiptLags
               /\ UNCHANGED <<receiptChannel, receiptVersion,
                              receiptPrevious>>
    /\ phase' = "idle"
    /\ UNCHANGED <<installed, initInst, everInstalled, txChannel,
                   txStartInstall, catVer, tampered, stagedHere, actVer,
                   actVerified, actChannel, actBase, publishes,
                   digestRefusals, interrupts, managerRefusals,
                   unknownRefusals, staleNoops>>

\* SIGTERM / failure at any point before the rename lands: the staged
\* temp is cleaned up, the target is untouched.
Interrupt ==
    /\ phase \in {"resolved", "fetching", "downloaded", "verified",
                  "staged", "midwrite"}
    /\ interrupts' = IF interrupts < 2 THEN interrupts + 1 ELSE interrupts
    /\ phase' = "idle"
    /\ stagedHere' = FALSE
    /\ tampered' = FALSE
    /\ UNCHANGED <<installed, initInst, everInstalled, receiptChannel,
                   receiptVersion, receiptPrevious, txChannel,
                   txStartInstall, catVer, actVer, actVerified, actChannel,
                   actBase, publishes, digestRefusals, managerRefusals,
                   unknownRefusals, staleNoops, receiptLags>>

ResetTx ==
    /\ phase \in {"aborted", "refused"}
    /\ phase' = "idle"
    /\ UNCHANGED <<installed, initInst, everInstalled, receiptChannel,
                   receiptVersion, receiptPrevious, txChannel,
                   txStartInstall, catVer, tampered, stagedHere, actVer,
                   actVerified, actChannel, actBase, publishes,
                   digestRefusals, interrupts, managerRefusals,
                   unknownRefusals, staleNoops, receiptLags>>

Next ==
    \/ BeginTx
    \/ ResolveDecision
    \/ Download
    \/ Verify
    \/ Stage
    \/ PublishAtomic
    \/ PublishEarly
    \/ PublishInPlaceStart
    \/ FinishInPlace
    \/ ReceiptRefresh
    \/ Interrupt
    \/ ResetTx

vars == <<installed, initInst, everInstalled, receiptChannel,
          receiptVersion, receiptPrevious, phase, txChannel,
          txStartInstall, catVer, tampered, stagedHere, actVer,
          actVerified, actChannel, actBase, publishes, digestRefusals,
          interrupts, managerRefusals, unknownRefusals, staleNoops,
          receiptLags>>

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* VF16 named safety properties.                                          *)
(***************************************************************************)

\* The target always holds a complete known artifact or nothing.
InstalledComplete == installed \in {"none", "v1", "v2"}

\* A version beyond the initial one is installed only when its
\* activation passed digest verification.
VerifiedBeforeActivation ==
    installed # initInst =>
        /\ actVer = installed
        /\ actVerified

\* An activation was authorized only by a tarball receipt.
ChannelGate == installed # initInst => actChannel = "tarball"

\* An aborted/refused transaction changed nothing; an idle system holds
\* either the transaction's starting artifact or its activation.
NoClobberOnFailure ==
    /\ (phase \in {"aborted", "refused"} => installed = txStartInstall)
    /\ (phase = "idle" => installed \in {initInst, actVer})

\* The receipt only ever names a version that was actually installed.
ReceiptNeverAhead ==
    receiptVersion # "" => receiptVersion \in everInstalled

\* Every activation moved to a strictly newer version than its base.
StaleNeverPublishes == actVer # "" => Newer(actVer, actBase)

(***************************************************************************)
(* Non-vacuity witnesses: each must be REACHABLE in the honest model      *)
(* (checked as an expected-to-fail invariant over the coverage config).   *)
(***************************************************************************)

\* A full verified update published (v1 -> v2, or a fresh install).
WitnessNoUpdate == actVer = ""

\* A tampered download was refused at the digest check.
WitnessNoDigestRefusal == digestRefusals = 0

\* An interruption mid-transaction preserved the old installation.
WitnessNoInterrupt == interrupts = 0

\* A manager-owned install was refused with routing.
WitnessNoManagerRefusal == managerRefusals = 0

\* A missing/malformed receipt was refused as an honest unknown.
WitnessNoUnknownRefusal == unknownRefusals = 0

\* A stale catalog produced a truthful no-op.
WitnessNoStaleNoop == staleNoops = 0

\* A successful update carried the superseded version forward.
WitnessNoReceiptCarry == receiptPrevious = ""

\* A publish succeeded while its receipt refresh failed (honest lag).
WitnessNoReceiptLag == receiptLags = 0

\* A fresh install (no old binary) activated.
WitnessNoFreshInstall ==
    ~(actVer # "" /\ actBase = "none")

=============================================================================
