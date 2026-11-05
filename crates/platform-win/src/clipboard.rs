use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};

const CF_UNICODETEXT: u32 = 13;

struct Clipboard;

impl Clipboard {
    fn open() -> Option<Self> {
        // The clipboard is a shared, single-owner resource: another process can
        // hold it for a few milliseconds, so a single attempt is not enough.
        for _ in 0..10 {
            if unsafe { OpenClipboard(HWND::default()) } != 0 {
                return Some(Self);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

pub fn set_text(text: &str) -> bool {
    let mut wide: Vec<u16> = std::ffi::OsStr::new(text).encode_wide().collect();
    wide.push(0);

    let Some(_guard) = Clipboard::open() else {
        return false;
    };
    unsafe {
        EmptyClipboard();
        let bytes = std::mem::size_of_val(&wide[..]);
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if handle.is_null() {
            return false;
        }
        let dst = GlobalLock(handle);
        if dst.is_null() {
            GlobalFree(handle);
            return false;
        }
        std::ptr::copy_nonoverlapping(wide.as_ptr(), dst.cast::<u16>(), wide.len());
        GlobalUnlock(handle);

        // The system only takes ownership of the block once SetClipboardData
        // succeeds; on failure it is still ours to release.
        if SetClipboardData(CF_UNICODETEXT, handle as HANDLE).is_null() {
            GlobalFree(handle);
            return false;
        }
        true
    }
}

pub fn get_text() -> Option<String> {
    let _guard = Clipboard::open()?;
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT) == 0 {
            return None;
        }
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() {
            return None;
        }
        let ptr = GlobalLock(handle as _).cast::<u16>();
        if ptr.is_null() {
            return None;
        }
        // CF_UNICODETEXT is required to be NUL-terminated, but a misbehaving
        // producer would otherwise walk us past the allocation.
        let cap = GlobalSize(handle as _) / 2;
        let mut len = 0;
        while len < cap && *ptr.add(len) != 0 {
            len += 1;
        }
        let text = OsString::from_wide(std::slice::from_raw_parts(ptr, len));
        GlobalUnlock(handle as _);
        text.into_string().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ignored by default: the clipboard is shared with whoever is using the
    // machine, and only text can be put back. Run it with
    // `cargo test -p platform-win -- --ignored`.
    #[test]
    #[ignore]
    fn round_trip_preserves_text() {
        let previous = get_text();

        let sample = "hello world\nsecond line\ttab · 中文 · 😀";
        assert!(set_text(sample), "could not write to the clipboard");
        assert_eq!(get_text().as_deref(), Some(sample));

        assert!(set_text(""), "empty text");
        assert_eq!(get_text().as_deref(), Some(""));

        if let Some(previous) = previous {
            set_text(&previous);
        }
    }
}
