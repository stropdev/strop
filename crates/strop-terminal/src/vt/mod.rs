//! Single-worker ownership of the pinned VT engine. Native pointers never cross
//! publication; only immutable Rust snapshots and bounded effects leave here.
mod ffi;
mod frame;
mod input;

#[cfg(test)]
mod tests;

use crate::model::*;
use crate::projection::Projection;
use crate::Error;
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

#[derive(Default)]
struct Events {
    reply: Vec<u8>,
    title: bool,
    directory: bool,
    bell: bool,
    clipboard_denied: bool,
    overflow: bool,
}

#[derive(Default)]
pub(crate) struct Emission {
    pub reply: Vec<u8>,
    pub effects: Vec<Effect>,
}

pub(crate) struct Vt {
    handle: NonNull<c_void>,
    events: Box<RefCell<Events>>,
    session: SessionId,
    geometry: Geometry,
    revision: u64,
    projection: Projection,
    screen: Vec<Arc<Row>>,
    palette: Option<Arc<Palette>>,
    native_history: usize,
    alternate: bool,
    rebuild: bool,
    history_limited: bool,
}

impl Vt {
    pub fn new(
        session: SessionId,
        geometry: Geometry,
        palette: Option<&Palette>,
    ) -> Result<Self, Error> {
        if !geometry.valid() {
            return Err(Error::Capacity("invalid terminal geometry"));
        }
        let events = Box::new(RefCell::new(Events::default()));
        let mut handle = std::ptr::null_mut();
        // SAFETY: the boxed callback state has a stable address through native
        // destruction. The native constructor owns/frees partial initialization.
        let result = unsafe {
            ffi::strop_vt_new(
                geometry.columns,
                geometry.rows,
                MAX_HISTORY_LINES,
                MAX_HISTORY_BYTES,
                (&*events as *const RefCell<Events>).cast_mut().cast(),
                event,
                &mut handle,
            )
        };
        check("create", result)?;
        let handle = NonNull::new(handle)
            .ok_or_else(|| Error::Protocol("native constructor returned no terminal".into()))?;
        let vt = Self {
            handle,
            events,
            session,
            geometry,
            revision: 0,
            projection: Projection::default(),
            screen: Vec::new(),
            palette: None,
            native_history: 0,
            alternate: false,
            rebuild: true,
            history_limited: false,
        };
        // The embedder-owned default palette (0065 D3) lands before the
        // first byte: default colors are strop's from birth, never a
        // ghostty default that later swaps.
        if let Some(palette) = palette {
            vt.configure_palette(palette)?;
        }
        Ok(vt)
    }

    fn configure_palette(&self, palette: &Palette) -> Result<(), Error> {
        if palette.colors.len() != 256 {
            return Err(Error::Capacity("terminal palette must hold 256 colors"));
        }
        let rgb = |value: Rgb| ffi::Rgb {
            r: value.red,
            g: value.green,
            b: value.blue,
        };
        let mut native = ffi::Palette {
            foreground: rgb(palette.foreground),
            background: rgb(palette.background),
            ..Default::default()
        };
        for (target, source) in native.colors.iter_mut().zip(&palette.colors) {
            *target = rgb(*source);
        }
        // SAFETY: live unique terminal; the native call copies the palette
        // synchronously and retains no pointer into it.
        check("palette set", unsafe {
            ffi::strop_vt_palette_set(self.handle.as_ptr(), &native)
        })
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Emission, Error> {
        if bytes.len() > 64 * 1024 {
            return Err(Error::Capacity("VT input quantum"));
        }
        // SAFETY: live unique terminal and immutable input bytes for this call.
        check("feed", unsafe {
            ffi::strop_vt_feed(self.handle.as_ptr(), bytes.as_ptr(), bytes.len())
        })?;
        self.take_events()
    }

    pub fn resize(&mut self, geometry: Geometry) -> Result<(), Error> {
        if !geometry.valid() {
            return Err(Error::Capacity("invalid terminal geometry"));
        }
        if geometry == self.geometry {
            return Ok(());
        }
        // SAFETY: dimensions were checked; no native cell borrow survives a call.
        check("resize", unsafe {
            ffi::strop_vt_resize(self.handle.as_ptr(), geometry.columns, geometry.rows)
        })?;
        self.geometry = geometry;
        self.rebuild = true;
        Ok(())
    }

    fn take_events(&mut self) -> Result<Emission, Error> {
        let mut events = self.events.borrow_mut();
        if events.overflow {
            return Err(Error::State("callback data exceeded its response budget"));
        }
        let reply = std::mem::take(&mut events.reply);
        let title = std::mem::take(&mut events.title);
        let directory = std::mem::take(&mut events.directory);
        let bell = std::mem::take(&mut events.bell);
        let clipboard = std::mem::take(&mut events.clipboard_denied);
        drop(events);
        let mut effects = Vec::new();
        if title {
            let bytes = self.metadata(1)?;
            let text =
                strop_core::layout::printable_text(String::from_utf8_lossy(&bytes)).into_owned();
            effects.push(Effect::Title(text));
        }
        if directory {
            effects.push(Effect::ReportedDirectory(self.metadata(2)?));
        }
        if bell {
            effects.push(Effect::Bell);
        }
        if clipboard {
            effects.push(Effect::ClipboardWriteDenied);
        }
        Ok(Emission { reply, effects })
    }

    fn metadata(&self, kind: u32) -> Result<Vec<u8>, Error> {
        let mut bytes = vec![0; 4096];
        let mut written = 0;
        // SAFETY: native pointer is live; storage has the declared capacity.
        check("metadata", unsafe {
            ffi::strop_vt_text(
                self.handle.as_ptr(),
                kind,
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut written,
            )
        })?;
        if written > bytes.len() {
            return Err(Error::Capacity("terminal metadata"));
        }
        bytes.truncate(written);
        Ok(bytes)
    }
}

impl Drop for Vt {
    fn drop(&mut self) {
        // SAFETY: this is the unique native owner, callbacks are synchronous,
        // and boxed callback state remains alive until after native destruction.
        unsafe { ffi::strop_vt_free(self.handle.as_ptr()) };
    }
}

fn check(operation: &'static str, code: i32) -> Result<(), Error> {
    match code {
        0 => Ok(()),
        -900 | -901 => Err(Error::Native {
            operation: "allocation budget",
            code,
        }),
        _ => Err(Error::Native { operation, code }),
    }
}

// SAFETY: only a live Vt installs this callback with its boxed RefCell pointer.
// Native callbacks run synchronously on that owner's thread and never re-enter
// the VT engine. Pointer bytes are copied only during their documented lifetime.
unsafe extern "C" fn event(userdata: *mut c_void, kind: u32, bytes: *const u8, len: usize) {
    // SAFETY: constructor-provided stable pointer, valid until native free returns.
    let state = unsafe { &*(userdata as *const RefCell<Events>) };
    let mut state = state.borrow_mut();
    match kind {
        0 => {
            if len > MAX_INPUT_BYTES.saturating_sub(state.reply.len())
                || (len != 0 && bytes.is_null())
            {
                state.overflow = true;
                return;
            }
            if len != 0 {
                // SAFETY: callback contract supplies len initialized bytes for this call.
                state
                    .reply
                    .extend_from_slice(unsafe { std::slice::from_raw_parts(bytes, len) });
            }
        }
        1 => state.title = true,
        2 => state.directory = true,
        3 => state.bell = true,
        4 => state.clipboard_denied = true,
        _ => state.overflow = true,
    }
}
