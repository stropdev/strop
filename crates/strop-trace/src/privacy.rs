//! Synchronous content classification before a producer constructs its payload.
use std::cell::Cell;

thread_local! {
    static CONTENT_ALLOWED: Cell<bool> = const { Cell::new(true) };
}

pub(crate) fn content_allowed() -> bool {
    CONTENT_ALLOWED.with(Cell::get)
}

/// Preserve metadata while withholding content in this synchronous producer.
/// Spawned jobs establish their own policy; the private guard cannot escape this
/// closure, and nesting or unwinding restores the caller's classification.
pub fn without_content<T>(produce: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONTENT_ALLOWED.with(|allowed| allowed.set(self.0));
        }
    }
    let previous = CONTENT_ALLOWED.with(|allowed| allowed.replace(false));
    let _restore = Restore(previous);
    produce()
}
