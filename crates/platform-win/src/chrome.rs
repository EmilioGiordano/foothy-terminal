use windows_sys::Win32::Foundation::{BOOL, HWND};
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW,
    WS_EX_LAYERED,
};

const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const DWMWA_USE_IMMERSIVE_DARK_MODE_PRE_20H1: u32 = 19;

/// Paints the title bar dark. Windows 10 changed the attribute id in 20H1, so
/// both are tried and the older one is the fallback.
pub fn set_dark_titlebar(hwnd: HWND, dark: bool) {
    let flag: BOOL = dark.into();
    let ptr = std::ptr::from_ref(&flag).cast();
    let size = size_of::<BOOL>() as u32;
    unsafe {
        if DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, ptr, size) != 0 {
            DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE_PRE_20H1, ptr, size);
        }
    }
}

/// Uniform window translucency. 255 is opaque.
pub fn set_opacity(hwnd: HWND, alpha: u8) {
    if alpha == 255 {
        return;
    }
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED as isize);
        SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
    }
}
