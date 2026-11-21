//! Publication and refresh decisions, after cache lookup. The host binds native
//! resource tokens to observed paths, content tokens to the bytes actually read,
//! and live attachments to their actual project, server and capabilities. A stat,
//! display name or language declaration is not such a witness. Generation tokens
//! are not reused while any delivery bearing them can still arrive.

/// A deletion or root switch retires the incarnation, even if the name returns.
#[derive(Clone, Copy)]
pub struct Resource {
    pub root: u64,
    pub root_incarnation: u64,
    pub path: u64,
    pub path_incarnation: u64,
}

fn same_resource(a: &Resource, b: &Resource) -> bool {
    a.root == b.root
        && a.root_incarnation == b.root_incarnation
        && a.path == b.path
        && a.path_incarnation == b.path_incarnation
}

/// The worker returns its captured stamp, not a stamp minted on delivery.
/// `schema` includes the format, extraction query and span convention.
pub struct AnswerStamp {
    pub resource: Resource,
    pub content: u64,
    pub child: u64,
    pub language: u64,
    pub schema: u64,
    pub query: u64,
    pub declaration: u64,
    pub scan: u64,
}

fn same_answer(a: &AnswerStamp, b: &AnswerStamp) -> bool {
    same_resource(&a.resource, &b.resource)
        && a.content == b.content
        && a.child == b.child
        && a.language == b.language
        && a.schema == b.schema
        && a.query == b.query
        && a.declaration == b.declaration
        && a.scan == b.scan
}

/// Source ownership for this particular surface, not just whether a file is open.
#[derive(Clone, Copy)]
pub enum Surface {
    Disk,
    BufferPending,
    BufferEmpty,
    BufferRows,
}

#[derive(Clone, Copy)]
pub enum Origin {
    Disk,
    Buffer,
}

/// Check the origin of an actual delivery. A pending buffer has no rows to
/// offer *yet*, but must be able to accept its first verified buffer reply.
/// Neither a pending buffer nor a verified empty one can borrow disk rows.
pub fn admits_origin(surface: Surface, origin: Origin) -> bool {
    match origin {
        Origin::Disk => matches!(surface, Surface::Disk),
        Origin::Buffer => matches!(
            surface,
            Surface::BufferPending | Surface::BufferEmpty | Surface::BufferRows
        ),
    }
}

/// Observations of the *designated* live binding. A restarted server or a
/// document rebind advances its respective token even at an unchanged path.
pub struct Attachment {
    pub root: u64,
    pub root_incarnation: u64,
    pub child: u64,
    pub language: u64,
    pub server: u64,
    pub attachment: u64,
    pub trusted: bool,
    pub capable: bool,
}

pub enum Binding {
    Unresolved,
    LanguageHint(u64),
    Live(Attachment),
}

/// Exactly one producer is designated for the request. In particular, an
/// unresolved semantic request cannot silently use a syntax shard instead.
pub enum Designation {
    Text(u64),
    Syntax(u64),
    Semantic(Binding),
}

pub enum Producer {
    Text(u64),
    Syntax(u64),
    Semantic { server: u64, attachment: u64 },
}

fn admits_producer(target: &AnswerStamp, want: &Designation, used: &Producer) -> bool {
    match (want, used) {
        (Designation::Text(a), Producer::Text(b)) => a == b,
        (Designation::Syntax(a), Producer::Syntax(b)) => a == b,
        (Designation::Semantic(binding), Producer::Semantic { server, attachment }) => {
            match binding {
                Binding::Unresolved => false,
                Binding::LanguageHint(_) => false,
                Binding::Live(live) => {
                    live.trusted
                        && live.capable
                        && live.root == target.resource.root
                        && live.root_incarnation == target.resource.root_incarnation
                        && live.child == target.child
                        && live.language == target.language
                        && live.server == *server
                        && live.attachment == *attachment
                }
            }
        }
        _ => false,
    }
}

pub struct Target {
    pub stamp: AnswerStamp,
    pub surface: Surface,
    pub designated: Designation,
}

pub struct Answer {
    pub stamp: AnswerStamp,
    pub origin: Origin,
    pub producer: Producer,
}

/// Gate display and action against the current target, not the target at
/// dispatch. `None` means there is no live request (e.g. cancellation).
pub fn admit(current: Option<&Target>, answer: &Answer) -> bool {
    match current {
        None => false,
        Some(t) => {
            same_answer(&t.stamp, &answer.stamp)
                && admits_origin(t.surface, answer.origin)
                && admits_producer(&t.stamp, &t.designated, &answer.producer)
        }
    }
}

/// The owner captures a separate obligation per resource/scope. Starting a new
/// walk, invalidating its subject, or losing/regaining watch coverage retires
/// the corresponding epoch. A completion cannot manufacture a new obligation.
#[derive(Clone, Copy)]
pub struct Scan {
    pub resource: Resource,
    pub declaration: u64,
    pub scan: u64,
    pub invalidation: u64,
    pub watch: u64,
}

#[derive(Clone, Copy)]
pub enum Refresh {
    Retired,
    Unwatched,
    Pending(Scan),
    Clean(Scan),
}

/// Discharge only a successful walk's own pending obligation. Return the
/// owner's current state unchanged on every rejected or redundant delivery.
pub fn settle(current: Refresh, started: &Scan, success: bool) -> Refresh {
    if let Refresh::Pending(now) = current {
        if success
            && same_resource(&now.resource, &started.resource)
            && now.declaration == started.declaration
            && now.scan == started.scan
            && now.invalidation == started.invalidation
            && now.watch == started.watch
        {
            return Refresh::Clean(now);
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESOURCE: Resource = Resource {
        root: 1,
        root_incarnation: 2,
        path: 3,
        path_incarnation: 4,
    };

    fn stamp() -> AnswerStamp {
        AnswerStamp {
            resource: RESOURCE,
            content: 5,
            child: 6,
            language: 7,
            schema: 8,
            query: 9,
            declaration: 10,
            scan: 11,
        }
    }

    #[test]
    fn held_reply_cannot_publish_after_rebind_or_cancel() {
        let mut target = Target {
            stamp: stamp(),
            surface: Surface::BufferPending,
            designated: Designation::Semantic(Binding::Live(Attachment {
                root: RESOURCE.root,
                root_incarnation: RESOURCE.root_incarnation,
                child: 6,
                language: 7,
                server: 12,
                attachment: 13,
                trusted: true,
                capable: true,
            })),
        };
        let held = Answer {
            stamp: stamp(),
            origin: Origin::Buffer,
            producer: Producer::Semantic {
                server: 12,
                attachment: 13,
            },
        };
        assert!(admit(Some(&target), &held));
        let Designation::Semantic(Binding::Live(binding)) = &mut target.designated else {
            unreachable!();
        };
        binding.attachment += 1;
        assert!(!admit(Some(&target), &held));
        assert!(!admit(None, &held));
    }

    #[test]
    fn out_of_order_scans_settle_only_their_captured_obligation() {
        let first = Scan {
            resource: RESOURCE,
            declaration: 1,
            scan: 2,
            invalidation: 3,
            watch: 4,
        };
        let second = Scan {
            invalidation: first.invalidation + 1,
            ..first
        };
        let pending = settle(Refresh::Pending(second), &first, true);
        assert!(matches!(pending, Refresh::Pending(_)));
        let clean = settle(pending, &second, true);
        assert!(
            matches!(settle(clean, &first, true), Refresh::Clean(scan) if scan.invalidation == second.invalidation)
        );
        assert!(matches!(
            settle(Refresh::Retired, &second, true),
            Refresh::Retired
        ));
        assert!(matches!(
            settle(Refresh::Unwatched, &second, true),
            Refresh::Unwatched
        ));
    }
}
