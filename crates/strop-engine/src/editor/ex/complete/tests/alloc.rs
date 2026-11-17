//! Plan 0110's performance rule as an assertion: once the first key has
//! warmed it, completing a command name allocates nothing per key.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use super::editor;

/// Counts this thread's allocations while `COUNTING` is set; every
/// other thread and every other moment pays one TLS read.
struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: every call forwards to `System` unchanged; the counter is a
// const-initialised thread-local `Cell`, which never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            COUNT.with(|count| count.set(count.get() + 1));
        }
        // SAFETY: the caller's contract is System's contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `alloc` above, which is System's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.with(Cell::get) {
            COUNT.with(|count| count.set(count.get() + 1));
        }
        // SAFETY: as for `alloc`; `ptr` and `layout` are System's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations(run: impl FnOnce()) -> usize {
    COUNT.with(|count| count.set(0));
    COUNTING.with(|counting| counting.set(true));
    run();
    COUNTING.with(|counting| counting.set(false));
    COUNT.with(Cell::get)
}

#[test]
fn completing_a_command_name_allocates_nothing_after_the_first_key() {
    for word in ["substitute", "wri", "tloc", "select-next", "%sor", "g/x/de"] {
        let mut e = editor();
        e.feed_text(":");
        // the first letter of the name is the first key of completion
        let first = word.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(0);
        for (index, c) in word.chars().enumerate() {
            let mut buffer = [0u8; 4];
            e.cmdline
                .line_mut()
                .expect("open line")
                .insert(c.encode_utf8(&mut buffer));
            let count = allocations(|| super::super::typed(&mut e));
            if index > first {
                assert_eq!(count, 0, "{word}: key {} allocated", index + 1);
            }
        }
        assert!(e.cmdline_menu().is_some(), "{word}: the menu shows");
    }
}
