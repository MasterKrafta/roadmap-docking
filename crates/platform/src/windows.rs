//! Windows-specific operations for GPUI-backed native windows.
//!
//! Call these functions on the UI thread and only while the source GPUI windows
//! remain alive. This module deliberately does not include the example's
//! high-resolution timer or its private GPUI redraw-message workaround.

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::ClientToScreen,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
        WindowsAndMessaging::*,
    },
};

/// Opaque native handle obtained from a live GPUI window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeWindowHandle(isize);

impl NativeWindowHandle {
    /// Returns the Win32 handle when the window is backed by a Win32 window.
    pub fn from_window(window: &Window) -> Option<Self> {
        match HasWindowHandle::window_handle(window).ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(Self(handle.hwnd.get())),
            _ => None,
        }
    }

    fn as_hwnd(self) -> HWND {
        HWND(self.0 as _)
    }
}

/// Returns the cursor position in screen coordinates.
pub fn cursor_position() -> (i32, i32) {
    let mut point = POINT::default();

    // SAFETY: point is a valid writable POINT; GetCursorPos fills it.
    unsafe {
        let _ = GetCursorPos(&mut point);
    }

    (point.x, point.y)
}

/// Returns whether the left mouse button is currently held.
pub fn left_button_down() -> bool {
    // SAFETY: GetAsyncKeyState accepts a virtual-key code and dereferences no pointers.
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16) & 0x8000 != 0 }
}

/// Returns the native window rectangle in screen coordinates.
pub fn window_rect(handle: NativeWindowHandle) -> Option<(i32, i32, i32, i32)> {
    let mut rect = RECT::default();

    // SAFETY: handle originates from GPUI; callers must not use it after its window closes.
    if unsafe { GetWindowRect(handle.as_hwnd(), &mut rect) }.is_err() {
        return None;
    }

    Some((rect.left, rect.top, rect.right, rect.bottom))
}

/// Returns the client area's top-left corner in screen coordinates.
pub fn client_origin(handle: NativeWindowHandle) -> Option<(i32, i32)> {
    let mut point = POINT::default();

    // SAFETY: handle originates from GPUI and point is a valid writable POINT.
    let succeeded = unsafe { ClientToScreen(handle.as_hwnd(), &mut point) }.as_bool();
    if !succeeded {
        return None;
    }

    Some((point.x, point.y))
}

/// Returns the native window's DPI scale factor.
pub fn scale_factor(handle: NativeWindowHandle) -> f32 {
    // SAFETY: handle originates from GPUI; callers must not use it after its window closes.
    let dpi = unsafe { GetDpiForWindow(handle.as_hwnd()) };

    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// Moves a window without changing its size or activating it.
pub fn move_window(handle: NativeWindowHandle, x: i32, y: i32) -> bool {
    // SAFETY: handle originates from GPUI; callers must not use it after its window closes.
    unsafe {
        SetWindowPos(
            handle.as_hwnd(),
            Some(HWND_TOP),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        )
    }
    .is_ok()
}

/// Resizes a window without changing its position or z-order.
pub fn resize_window(handle: NativeWindowHandle, width: i32, height: i32) -> bool {
    // SAFETY: handle originates from GPUI; callers must not use it after its window closes.
    unsafe {
        SetWindowPos(
            handle.as_hwnd(),
            None,
            0,
            0,
            width,
            height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
    .is_ok()
}

/// Shows a window without activating it.
pub fn show_window(handle: NativeWindowHandle) {
    // SAFETY: handle originates from GPUI; callers must not use it after its window closes.
    unsafe {
        let _ = ShowWindow(handle.as_hwnd(), SW_SHOWNOACTIVATE);
    }
}

/// Hides a window.
pub fn hide_window(handle: NativeWindowHandle) {
    // SAFETY: handle originates from GPUI; callers must not use it after its window closes.
    unsafe {
        let _ = ShowWindow(handle.as_hwnd(), SW_HIDE);
    }
}

/// Restores and activates a window after its last owned floating panel closes.
pub fn restore_and_activate(handle: NativeWindowHandle) {
    // SAFETY: handle originates from a live GPUI window.
    unsafe {
        let _ = ShowWindow(handle.as_hwnd(), SW_RESTORE);
        let _ = SetForegroundWindow(handle.as_hwnd());
    }
}

/// Makes owner the owner of window, so the floating window follows its main window.
pub fn set_owner(window: NativeWindowHandle, owner: NativeWindowHandle) {
    // SAFETY: Both handles must come from live GPUI windows.
    unsafe {
        let _previous = SetWindowLongPtrW(window.as_hwnd(), GWLP_HWNDPARENT, owner.0);
    }
}

/// Removes the always-on-top state without changing the window's owner.
///
/// Owned windows still stay above their owner, but can be covered by windows
/// from other applications when the user switches away.
pub fn remove_topmost(handle: NativeWindowHandle) -> bool {
    // SAFETY: handle originates from GPUI; callers must not use it after its window closes.
    unsafe {
        SetWindowPos(
            handle.as_hwnd(),
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    }
    .is_ok()
}

/// Enables the native Windows resize frame on a borderless GPUI window.
pub fn enable_native_resize(handle: NativeWindowHandle) {
    // SAFETY: handle originates from GPUI; callers must not use it after its window closes.
    unsafe {
        let hwnd = handle.as_hwnd();
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 | WS_THICKFRAME.0;
        let _previous = SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);

        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}
