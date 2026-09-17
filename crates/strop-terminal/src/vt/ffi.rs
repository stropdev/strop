//! The only Rust declarations of the pinned native ABI. The owning VT wrapper
//! bounds every buffer and keeps all native references on one worker thread.
use std::ffi::{c_char, c_void};

#[repr(C)]
#[derive(Default)]
pub(super) struct Cell {
    pub codepoint: u32,
    pub graphemes: u32,
    pub width: u32,
    pub fg_kind: u32,
    pub fg: u32,
    pub bg_kind: u32,
    pub bg: u32,
    pub flags: u32,
    pub wrapped: u32,
}

#[repr(C)]
#[derive(Default)]
pub(super) struct State {
    pub cols: u64,
    pub rows: u64,
    pub cursor_x: u64,
    pub cursor_y: u64,
    pub visible: u64,
    pub blinking: u64,
    pub cursor_style: u64,
    pub alternate: u64,
    pub history: u64,
    pub modes: u64,
    pub memory_bytes: u64,
    pub previous_history_anchor: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[repr(C)]
pub(super) struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub colors: [Rgb; 256],
}
impl Default for Palette {
    fn default() -> Self {
        Self {
            foreground: Rgb::default(),
            background: Rgb::default(),
            colors: [Rgb::default(); 256],
        }
    }
}

pub(super) type EventFn = unsafe extern "C" fn(*mut c_void, u32, *const u8, usize);

unsafe extern "C" {
    pub fn strop_vt_new(
        columns: u16,
        rows: u16,
        history_lines: usize,
        history_bytes: usize,
        userdata: *mut c_void,
        event: EventFn,
        out: *mut *mut c_void,
    ) -> i32;
    pub fn strop_vt_free(handle: *mut c_void);
    pub fn strop_vt_feed(handle: *mut c_void, data: *const u8, len: usize) -> i32;
    pub fn strop_vt_resize(handle: *mut c_void, columns: u16, rows: u16) -> i32;
    pub fn strop_vt_state(handle: *mut c_void, out: *mut State) -> i32;
    pub fn strop_vt_dirty_rows(handle: *mut c_void, out: *mut u8, capacity: usize) -> i32;
    pub fn strop_vt_palette(handle: *mut c_void, out: *mut Palette) -> i32;
    pub fn strop_vt_palette_set(handle: *mut c_void, palette: *const Palette) -> i32;
    pub fn strop_vt_cell(handle: *mut c_void, row: i32, column: u16, out: *mut Cell) -> i32;
    pub fn strop_vt_graphemes(
        handle: *mut c_void,
        row: i32,
        column: u16,
        out: *mut u32,
        capacity: usize,
        written: *mut usize,
    ) -> i32;
    pub fn strop_vt_mark_history(handle: *mut c_void) -> i32;
    pub fn strop_vt_key(
        handle: *mut c_void,
        name: *const c_char,
        scalar: u32,
        modifiers: u32,
        action: u32,
        state: u32,
        text: *const c_char,
        len: usize,
        out: *mut u8,
        capacity: usize,
        written: *mut usize,
    ) -> i32;
    pub fn strop_vt_paste(handle: *mut c_void, data: *const u8, len: usize, confirmed: i32) -> i32;
    pub fn strop_vt_focus(
        handle: *mut c_void,
        focused: i32,
        out: *mut u8,
        capacity: usize,
        written: *mut usize,
    ) -> i32;
    pub fn strop_vt_keyboard(handle: *mut c_void, allowed: u8) -> i32;
    pub fn strop_vt_text(
        handle: *mut c_void,
        kind: u32,
        out: *mut u8,
        capacity: usize,
        written: *mut usize,
    ) -> i32;
}
