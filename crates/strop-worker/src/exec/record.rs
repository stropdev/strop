//! A child process's observed exit state. The worker emits it through
//! a typed protocol result, never through mixed child output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusRecord {
    /// The target exited with this code. Non-zero codes are data, not
    /// transport failures.
    Exited(u32),
    /// The target died on this signal.
    Signaled(u32),
}
