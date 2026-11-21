//! Candidate scheduling and completion of a policy-aware project query.
//! The host owns the authoritative walk, binds observations to non-reused
//! tokens, and keeps one ledger entry per required child and per eligible
//! path. These decisions inspect those observations, not a retained hit list.

/// Only current walk/policy or exact verification evidence can exclude a path.
/// An old negative is `Unknown`, not `Excluded`.
#[derive(Clone, Copy)]
pub enum Evidence {
    Unknown,
    Eligible,
    Excluded,
}

/// Declarations and cache hits are scheduling evidence, not walk policy.
#[derive(Clone, Copy)]
pub enum Hint {
    None,
    LikelyHit,
    Declared { project: u64, language: Option<u64> },
}

#[derive(Clone, Copy)]
pub enum Schedule {
    Skip,
    Examine,
    ExamineFirst,
}

pub fn schedule(evidence: Evidence, hint: Hint) -> Schedule {
    match evidence {
        Evidence::Excluded => Schedule::Skip,
        Evidence::Unknown | Evidence::Eligible => match hint {
            Hint::None => Schedule::Examine,
            Hint::LikelyHit | Hint::Declared { .. } => Schedule::ExamineFirst,
        },
    }
}

/// Scope names the root incarnation and folder. Policy names the complete
/// selection and provider policy (text does not require a semantic server).
#[derive(Clone, Copy)]
pub struct Query {
    pub scope: u64,
    pub policy: u64,
    pub generation: u64,
}

#[derive(Clone, Copy)]
pub enum Terminal {
    Running,
    Success,
    Truncated,
    Failed,
    Unsupported,
}

/// One obligation created by the walk, not one result row. Verification can
/// settle a path with zero rows; queuing it or starting its verifier cannot.
#[derive(Clone, Copy)]
pub enum PathWork {
    Queued,
    Verifying,
    Verified,
}

/// Minted only after the walk and per-path verifier have both finished.
pub struct Certificate {
    query: Query,
    project: Option<u64>,
}

/// `Partial(Unsupported)` is unavailable semantic coverage, not zero rows.
pub enum Settlement {
    Partial(Terminal),
    Complete(Certificate),
}

/// A required child exists independently of whether its worker has replied.
pub struct Child {
    pub project: u64,
    pub scan: Settlement,
}

fn all_verified(paths: &[PathWork]) -> bool {
    let mut k = 0;
    while k < paths.len() {
        if !matches!(paths[k], PathWork::Verified) {
            return false;
        }
        k += 1;
    }
    true
}

pub fn settle(
    query: Query,
    project: Option<u64>,
    exhausted: bool,
    terminal: Terminal,
    paths: &[PathWork],
) -> Settlement {
    if matches!(terminal, Terminal::Success) && exhausted && all_verified(paths) {
        Settlement::Complete(Certificate { query, project })
    } else {
        Settlement::Partial(terminal)
    }
}

fn same_project(required: Option<u64>, observed: Option<u64>) -> bool {
    match (required, observed) {
        (None, None) => true,
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn current(query: &Query, project: Option<u64>, report: &Settlement) -> bool {
    let Settlement::Complete(scan) = report else {
        return false;
    };
    query.scope == scan.query.scope
        && query.policy == scan.query.policy
        && query.generation == scan.query.generation
        && same_project(project, scan.project)
}

/// `children` comes from the required scope, *not* from the list of workers
/// which happened to deliver. The loose-path scan also witnesses exhaustion
/// of the authoritative workspace walk (including discovery of children).
pub fn workspace_complete(query: &Query, loose: &Settlement, children: &[Child]) -> bool {
    if !current(query, None, loose) {
        return false;
    }
    let mut k = 0;
    while k < children.len() {
        let child = &children[k];
        if !current(query, Some(child.project), &child.scan) {
            return false;
        }
        k += 1;
    }
    true
}
