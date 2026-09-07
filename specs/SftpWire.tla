---- MODULE SftpWire ----
(* The SFTP v3 read-only wire protocol as shipped (0034) — the codec in   *)
(* crates/strop-remote/src/transport/wire.rs, driven stage-by-stage by    *)
(* crates/strop-remote/src/transport/session.rs and orchestrated by      *)
(* crates/strop-remote/src/transport.rs — modeled at the exchange         *)
(* boundary the Rust client has:                                          *)
(*                                                                       *)
(*   1. one connection, one outstanding request: begin() mints a          *)
(*      monotonic id (u32 checked_add, first request is 1; the            *)
(*      INIT/VERSION handshake itself carries no id) and exchange()      *)
(*      is strictly write-then-read, so the client never owes two        *)
(*      replies and must never resume on a reply minted for another      *)
(*      request                                                          *)
(*   2. framing before allocation: exchange() reads the response          *)
(*      length and rejects 0 or > MAX_PACKET BEFORE response.resize       *)
(*      — an out-of-bounds frame never becomes an allocation             *)
(*   3. inspect() proves the handle before any byte is buffered:          *)
(*      regular-file permissions, a present length, length <=            *)
(*      MAX_SNAPSHOT; that captured size is the budget the read loop     *)
(*      may never exceed (Vec::with_capacity(size), count =              *)
(*      min(READ_CHUNK, remaining))                                      *)
(*   4. every reply is validated — frame bound, reply id, shape,          *)
(*      status code, expected type, payload semantics — and every        *)
(*      failure is a typed Fault (Stage, Kind), never partial success    *)
(*   5. success is published only after UTF-8 validation and an          *)
(*      accepted CLOSE reply. The post-close cancellation race           *)
(*      (session.rs transfer(): token.is_cancelled() before Ok) is       *)
(*      owned by RemoteRead, not by this module                          *)
(*                                                                       *)
(* THE invariants — each names an executable Rust oracle boundary that   *)
(* Main owns (re-checking the same property against the real codec):     *)
(*                                                                       *)
(*   TypeOK                 every variable stays in its declared         *)
(*                          finite domain (supersets where mutants       *)
(*                          need headroom)                               *)
(*   ReplyIdentity          every reply consumed as an answer carried    *)
(*                          the id of the request it answered, and at    *)
(*                          most one request is ever unanswered          *)
(*                          (Rust: reply()'s "response belongs to a      *)
(*                          different request" check; oracle: feed the   *)
(*                          codec a stale/future-id reply and require    *)
(*                          a Protocol fault with no state advance)      *)
(*   PacketBounded          every response-buffer allocation lies in     *)
(*                          1..MAX_PACKET                                 *)
(*                          (Rust: exchange()'s length gate before       *)
(*                          resize; oracle: offer 0-length and           *)
(*                          MAX_PACKET+1-length frames, require fault    *)
(*                          and no oversized allocation)                 *)
(*   SnapshotBounded        bytes buffered never exceed the length       *)
(*                          captured at inspect, which never exceeds     *)
(*                          MAX_SNAPSHOT                                 *)
(*                          (Rust: read()'s count cap and "sent more     *)
(*                          bytes than requested" check; oracle: long    *)
(*                          Data replies must fault, snapshot length     *)
(*                          must equal the captured size)                *)
(*   CompleteBeforeSuccess  SUCCESS implies validation passed and        *)
(*                          CLOSE was accepted first                     *)
(*                          (Rust: session.rs stage ordering Connect ->  *)
(*                          Open -> Inspect -> Transfer -> Validate ->   *)
(*                          Teardown; oracle: assert close ran on the    *)
(*                          success path and validation before the       *)
(*                          buffer escapes)                              *)
(*                                                                       *)
(* Bytes are abstract on purpose: the model tracks counts, never          *)
(* contents. The file's UTF-8 decodability is the constant FILE_UTF8.    *)
(* Actual byte decoding, native path bytes, the handle-length limit      *)
(* (1..1024), FSTAT attribute subfields (uid/gid, times) and version     *)
(* extension strings are owned by the Rust codec and its oracles —       *)
(* they are collapsed here into the reply's shape dimension.             *)
(*                                                                       *)
(* Deliberately faulty variants are parameterized guard mutations on     *)
(* one constant knob (no copy-pasted mutant module); configs flip        *)
(* MUTATION:                                                            *)
(*   MUTATION = 1  accept a reply whose id is not the outstanding        *)
(*                 request's — must die by exactly ReplyIdentity        *)
(*   MUTATION = 2  accept Data longer than the request count             *)
(*                 (over-read) — must die by exactly SnapshotBounded    *)
(*   MUTATION = 3  allocate the frame before checking its bounds —       *)
(*                 must die by exactly PacketBounded                     *)
(*                                                                       *)
(* This follows rootle's provider-protocol precedent                     *)
(* (https://rootle.dev/docs/provider-protocol.html): a bounded TLA+      *)
(* model with safety invariants, deliberately faulty variants and        *)
(* executable checks over the real boundary — and no claim of            *)
(* unbounded soundness, of Rust byte decoding, of SSH transport          *)
(* integrity, or of server behavior beyond the replies enumerated        *)
(* here. Bounded model checking only; TLC explores the finite constants  *)
(* below.                                                                *)
(*                                                                       *)
(* Not modeled: transport integrity (SSH owns it), pipelining (the       *)
(* client is sequential by construction), request-id exhaustion (u32;    *)
(* MAXSEQ only bounds the finite state space), network timing.           *)
(***************************************************************************)
EXTENDS Integers, FiniteSets

CONSTANTS MAXPACKET,  \* response frame bound (Rust MAX_PACKET = 256 KiB, scaled)
          MAXSNAP,    \* snapshot byte cap (Rust MAX_SNAPSHOT = 256 MiB, scaled)
          CHUNK,      \* bytes per READ request (Rust READ_CHUNK = 32 KiB, scaled)
          FSIZE,      \* length FSTAT reports for the proven regular file
          FILE_UTF8,  \* whether the snapshot decodes as UTF-8 (Rust owns bytes)
          MAXSEQ,     \* request-id cap for finiteness (Rust u32 overflow is far)
          MUTATION    \* 0 honest; 1 wrong-id; 2 over-read; 3 unbounded alloc

VARIABLES phase,       \* client stage in the shipped read sequence
          seq,         \* id of the outstanding (or last) request; begins at 0
          outcome,     \* RUNNING until the session ends in SUCCESS or a Fault
          asked,       \* ghost: request ids begun (the client's monotonic ids)
          answered,    \* ghost: ids whose reply was consumed as an answer
          answeredBy,  \* ghost: id -> the reply id it was actually answered by
          allocations, \* ghost: every response-buffer allocation size
          cap,         \* snapshot capacity captured at inspect (0 before)
          got,         \* bytes buffered into the snapshot so far
          closedOk,    \* ghost: an ok CLOSE reply was consumed
          validated    \* ghost: the snapshot passed UTF-8 validation

vars == <<phase, seq, outcome, asked, answered, answeredBy, allocations, cap, got, closedOk, validated>>

\* -- stages ---------------------------------------------------------------
CONNECTING   == 1   \* awaiting VERSION (draft-ietf-secsh-filexfer-02 s4.1)
OPENING      == 2   \* awaiting HANDLE for SSH_FXP_OPEN (READ-only flags)
INSPECTING   == 3   \* awaiting ATTRS for SSH_FXP_FSTAT
TRANSFER     == 4   \* read loop: awaiting DATA for SSH_FXP_READ
VALIDATE     == 5   \* local UTF-8 check, no request outstanding
CLOSING      == 6   \* awaiting STATUS(ok) for SSH_FXP_CLOSE
TEARDOWN_OK  == 7   \* close accepted; success not yet published
DONE         == 8   \* terminal: SUCCESS or a Fault (absorbing)

PHASES == {CONNECTING, OPENING, INSPECTING, TRANSFER, VALIDATE, CLOSING, TEARDOWN_OK, DONE}
AWAITING == {CONNECTING, OPENING, INSPECTING, TRANSFER, CLOSING}

\* -- terminal fault atoms; the comment maps each to Rust (Stage, Kind) ---
RUNNING      == 0
SUCCESS      == 1
FRAMING      == 2    \* (exchange stage, Protocol): length outside 1..MAX_PACKET
TRUNCATED    == 3    \* (reply stage, Protocol): truncated SFTP response
TRAILING     == 4    \* (reply stage, Protocol): unexpected trailing bytes
WRONG_ID     == 5    \* (reply stage, Protocol): belongs to a different request
UNEXPECTED   == 6    \* (reply stage, Protocol): unexpected response type
BAD_VERSION  == 7    \* (Connect, Protocol): server must support version 3
STATUS       == 8    \* (stage, ShortRead|NotFound|Permission|Protocol): code
NOT_REGULAR  == 9    \* (Inspect, NotRegularFile)
NO_LENGTH    == 10   \* (Inspect, UnknownLength)
TOO_LARGE    == 11   \* (Inspect, TooLarge)
SHORT_READ   == 12   \* (Transfer, ShortRead): no data before captured end
OVER_LENGTH  == 13   \* (Transfer, Protocol): more bytes than requested
INVALID_UTF8 == 14   \* (Validate, InvalidUtf8)

OUTCOMES == {RUNNING, SUCCESS, FRAMING, TRUNCATED, TRAILING, WRONG_ID, UNEXPECTED,
             BAD_VERSION, STATUS, NOT_REGULAR, NO_LENGTH, TOO_LARGE, SHORT_READ,
             OVER_LENGTH, INVALID_UTF8}

\* -- reply dimensions (symbolic; numbers are only tags) -------------------
SHAPE_WF     == 0    \* payload decodes exactly (Decoder end() succeeds)
SHAPE_TRUNC  == 1    \* payload underruns a field (Decoder take fails)
SHAPE_TRAIL  == 2    \* payload has leftovers (Decoder end fails)
R_EXPECTED   == 0    \* the packet type reply() expects in this stage
R_STATUS     == 1    \* SSH_FXP_STATUS
R_WRONG      == 2    \* any other type
A_IRRELEVANT == 0    \* attrs field unused outside INSPECTING
A_REGULAR    == 1    \* permissions prove regular file, size present and legal
A_NOT_REGULAR== 2    \* permissions do not prove a regular file
A_NO_LENGTH  == 3    \* size flag absent
A_TOO_BIG    == 4    \* reported size exceeds MAX_SNAPSHOT
D_IRRELEVANT == 0    \* dlen field unused outside TRANSFER
D_ZERO       == 1    \* empty Data (short read)
D_ONE        == 2    \* one byte (a legal partial chunk)
D_FULL       == 3    \* exactly the requested count
D_OVER       == 4    \* count + 1 (the over-read hazard)

Min2(a, b) == IF a =< b THEN a ELSE b

\* The server is adversarial within these finite dimensions. rid is numeric:
\* the outstanding request's id (seq) or a wrong one (seq + 1 stands for any
\* stale or future id). The handshake carries no id, so rid is 0 there.
ReplyIds ==
    IF phase = CONNECTING THEN {0} ELSE {seq, seq + 1}

ReplyKinds ==
    IF phase \in {CONNECTING, CLOSING} THEN {R_EXPECTED, R_WRONG}
    ELSE {R_EXPECTED, R_STATUS, R_WRONG}

ReplySvals == {0, 1, 2, 3, 6, 7}
    \* 6: a version other than 3 (CONNECTING); 0: SSH_FX_OK or an abstract
    \* legal value; 1/2/3: error/NoSuchFile/PermissionDenied; 7: any other.

ReplyAttrs ==
    IF phase = INSPECTING
    THEN {A_REGULAR, A_NOT_REGULAR, A_NO_LENGTH, A_TOO_BIG}
    ELSE {A_IRRELEVANT}

ReplyDlens ==
    IF phase = TRANSFER THEN {D_ZERO, D_ONE, D_FULL, D_OVER} ELSE {D_IRRELEVANT}

PhaseReplies ==
    {reply \in
        [rid: ReplyIds, flen: {1, 0, MAXPACKET + 1},
         shape: {SHAPE_WF, SHAPE_TRUNC, SHAPE_TRAIL}, rkind: ReplyKinds,
         sval: ReplySvals, attrs: ReplyAttrs, dlen: ReplyDlens] :
        \/ reply.rkind = R_STATUS
        \/ phase \in {CONNECTING, CLOSING}
        \/ reply.sval = 0}
    \* flen 1 abstracts any length within 1..MAX_PACKET; 0 and MAX_PACKET+1
    \* are the framing violations exchange() must reject before allocating.

TypeOK ==
    /\ phase \in PHASES
    /\ seq \in 0..MAXSEQ
    /\ outcome \in OUTCOMES
    /\ asked \subseteq 1..MAXSEQ
    /\ answered \subseteq asked
    /\ DOMAIN answeredBy = answered
    /\ \A i \in answered : answeredBy[i] \in 0..MAXSEQ + 1
    /\ allocations \subseteq {0, 1, MAXPACKET + 1}
    /\ cap \in {0, FSIZE}
    /\ got \in 0..(MAXSNAP + CHUNK + 2)
    /\ closedOk \in BOOLEAN
    /\ validated \in BOOLEAN

Init ==
    /\ phase = CONNECTING
    /\ seq = 0
    /\ outcome = RUNNING
    /\ asked = {}
    /\ answered = {}
    /\ answeredBy = [x \in {} |-> 0]
    /\ allocations = {}
    /\ cap = 0
    /\ got = 0
    /\ closedOk = FALSE
    /\ validated = FALSE

\* -- shared action fragments ----------------------------------------------
\* A terminal fault: the session is over, nothing else moves. The caller
\* owns `allocations` and `phase`/`outcome` are set here.
Terminal(o) ==
    /\ outcome' = o
    /\ phase' = DONE
    /\ UNCHANGED <<seq, asked, answered, answeredBy, cap, got, closedOk, validated>>

\* Record only a fully accepted answer; EXCEPT cannot add a function-domain key.
RecordAnswer(reply) ==
    /\ answered' = answered \union {seq}
    /\ answeredBy' = [i \in answered \union {seq} |->
                       IF i = seq THEN reply.rid ELSE answeredBy[i]]

\* The handshake: connect() parses VERSION directly (no request id, no
\* reply()); anything but Version(3) with a clean payload is one fault.
ConnectBody(reply) ==
    IF reply.shape = SHAPE_TRUNC THEN Terminal(TRUNCATED)
    ELSE IF reply.shape = SHAPE_TRAIL THEN Terminal(TRAILING)
    ELSE IF reply.rkind /= R_EXPECTED \/ reply.sval /= 3 THEN Terminal(BAD_VERSION)
    ELSE /\ seq' = 1                       \* begin() for SSH_FXP_OPEN
         /\ asked' = asked \union {1}
         /\ phase' = OPENING
         /\ UNCHANGED <<outcome, answered, answeredBy, cap, got, closedOk, validated>>


\* Advance the read loop: `to` bytes buffered. Leaving the budget or
\* finishing the file ends the loop; otherwise the next READ begins.
Advance(to, reply) ==
    /\ RecordAnswer(reply)
    /\ got' = to
    /\ IF to >= cap
       THEN /\ phase' = VALIDATE
            /\ UNCHANGED <<seq, asked>>
       ELSE /\ seq < MAXSEQ
            /\ seq' = seq + 1
            /\ asked' = asked \union {seq + 1}
            /\ phase' = TRANSFER
    /\ UNCHANGED <<outcome, cap, closedOk, validated>>

TransferBody(reply) ==
    LET count == Min2(CHUNK, cap - got) IN
    CASE reply.dlen = D_ZERO -> Terminal(SHORT_READ)
      [] reply.dlen = D_OVER ->
           IF MUTATION = 2
           THEN Advance(got + count + 1, reply)    \* THE OVER-READ MUTATION
           ELSE Terminal(OVER_LENGTH)
      [] OTHER -> Advance(got + (IF reply.dlen = D_ONE THEN 1 ELSE count), reply)

PayloadBody(reply) ==
    CASE phase = OPENING ->
            /\ RecordAnswer(reply)
            /\ seq < MAXSEQ
            /\ seq' = seq + 1                     \* begin() for SSH_FXP_FSTAT
            /\ asked' = asked \union {seq + 1}
            /\ phase' = INSPECTING
            /\ UNCHANGED <<outcome, cap, got, closedOk, validated>>
      [] phase = INSPECTING ->
            CASE reply.attrs = A_REGULAR ->
                 /\ RecordAnswer(reply)
                 /\ cap' = FSIZE
                 /\ IF FSIZE = 0
                    THEN /\ phase' = VALIDATE
                         /\ UNCHANGED <<seq, asked>>
                    ELSE /\ seq < MAXSEQ
                         /\ seq' = seq + 1
                         /\ asked' = asked \union {seq + 1}
                         /\ phase' = TRANSFER
                 /\ UNCHANGED <<outcome, got, closedOk, validated>>
               [] reply.attrs = A_NOT_REGULAR -> Terminal(NOT_REGULAR)
               [] reply.attrs = A_NO_LENGTH -> Terminal(NO_LENGTH)
               [] OTHER -> Terminal(TOO_LARGE)
      [] phase = TRANSFER -> TransferBody(reply)
      [] phase = CLOSING ->
            IF reply.sval = 0
            THEN /\ RecordAnswer(reply)
                 /\ closedOk' = TRUE
                 /\ phase' = TEARDOWN_OK
                 /\ UNCHANGED <<outcome, seq, asked, cap, got, validated>>
            ELSE Terminal(STATUS)
      [] OTHER -> FALSE    \* unreachable: CONNECTING is routed to ConnectBody
\* Beyond the handshake, reply() checks (in order) the reply id, the
\* status encoding, the expected type, then the caller decodes the payload.
Body(reply) ==
    IF reply.shape = SHAPE_TRUNC THEN Terminal(TRUNCATED)
    ELSE IF reply.shape = SHAPE_TRAIL THEN Terminal(TRAILING)
    ELSE IF reply.rkind = R_STATUS
        THEN IF reply.sval = 0
             THEN Terminal(UNEXPECTED)   \* ok-status where another type is due
             ELSE Terminal(STATUS)
    ELSE IF reply.rkind /= R_EXPECTED THEN Terminal(UNEXPECTED)
    ELSE PayloadBody(reply)

\* id gate: the reply must belong to the outstanding request. MUTATION = 1
\* removes exactly this gate — the mismatched reply is then consumed and
\* recorded, and ReplyIdentity must catch it.
PostFrame(reply) ==
    IF phase = CONNECTING THEN ConnectBody(reply)
    ELSE IF reply.rid /= seq /\ MUTATION /= 1 THEN Terminal(WRONG_ID)
    ELSE Body(reply)

\* One exchange: the request is written, the framed response arrives, the
\* frame bound gates allocation (exchange()), then the reply is decoded.
ServerRespond(reply) ==
    /\ outcome = RUNNING
    /\ phase \in AWAITING
    /\ IF reply.flen = 0 \/ reply.flen > MAXPACKET
       THEN IF MUTATION = 3
            THEN /\ allocations' = allocations \union {reply.flen}
                 /\ PostFrame(reply)            \* THE UNBOUNDED-ALLOC MUTATION
            ELSE /\ allocations' = allocations   \* fault before any allocation
                 /\ Terminal(FRAMING)
       ELSE /\ allocations' = allocations \union {reply.flen}
            /\ PostFrame(reply)

\* Local validation: String::from_utf8 on exactly the captured bytes.
ValidateUtf8 ==
    /\ phase = VALIDATE
    /\ outcome = RUNNING
    /\ IF FILE_UTF8
       THEN /\ validated' = TRUE
            /\ seq < MAXSEQ
            /\ seq' = seq + 1                    \* begin() for SSH_FXP_CLOSE
            /\ asked' = asked \union {seq + 1}
            /\ phase' = CLOSING
            /\ UNCHANGED <<outcome, answered, answeredBy, allocations, cap, got, closedOk>>
       ELSE /\ outcome' = INVALID_UTF8
            /\ phase' = DONE
            /\ UNCHANGED <<seq, asked, answered, answeredBy, allocations, cap, got, closedOk, validated>>

\* Publication of the snapshot: only after close was accepted.
Complete ==
    /\ phase = TEARDOWN_OK
    /\ outcome = RUNNING
    /\ outcome' = SUCCESS
    /\ phase' = DONE
    /\ UNCHANGED <<seq, asked, answered, answeredBy, allocations, cap, got, closedOk, validated>>

Next ==
    \/ \E reply \in PhaseReplies : ServerRespond(reply)
    \/ ValidateUtf8
    \/ Complete

Spec == Init /\ [][Next]_vars

\* -- the invariants --------------------------------------------------------

ReplyIdentity ==
    /\ answered \subseteq asked
    /\ Cardinality(asked \ answered) =< 1
    /\ \A i \in DOMAIN answeredBy : answeredBy[i] = i

PacketBounded ==
    allocations \subseteq 1..MAXPACKET

SnapshotBounded ==
    /\ got =< cap
    /\ cap =< MAXSNAP

CompleteBeforeSuccess ==
    outcome = SUCCESS => (closedOk /\ validated)

\* -- coverage witnesses (checked only by the coverage configs; each      *)
(* must be VIOLATED so the model cannot pass vacuously)                   *)

WitnessNoSuccess == outcome /= SUCCESS
WitnessNoUtf8Failure == outcome /= INVALID_UTF8

\* Only MUTATION = 0 configurations are intended to satisfy the safety predicates.
\* These are bounded TLC checks, not an asserted universal theorem over constants.
=============================================================================
