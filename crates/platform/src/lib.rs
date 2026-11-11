//! Narrow native-window API used by the GPUI adapter.
//!
//! This crate is the only layer that may call Win32 directly. The GPUI
//! adapter remains safe Rust and depends only on the platform-neutral types
//! and trait.

#![deny(unsafe_op_in_unsafe_fn)]

use gpui::Window;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NativeWindowId(isize);

impl NativeWindowId {
    pub const fn from_raw(value: isize) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn as_raw(self) -> isize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PhysicalSize {
    pub width: i32,
    pub height: i32,
}

impl PhysicalSize {
    pub const fn is_valid(self) -> bool {
        self.width > 0 && self.height > 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PhysicalRect {
    pub const fn width(self) -> i32 {
        self.right - self.left
    }

    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub const fn contains(self, point: PhysicalPoint) -> bool {
        point.x >= self.left
            && point.x < self.right
            && point.y >= self.top
            && point.y < self.bottom
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeWindowAtPoint {
    Known(NativeWindowId),
    Foreign,
    Empty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeKey {
    Escape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeError {
    Unsupported,
    WindowHandleUnavailable,
    UnexpectedWindowHandle,
    InvalidSize,
    Platform {
        operation: &'static str,
        code: u32,
    },
}

impl fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("native window operations are unsupported"),
            Self::WindowHandleUnavailable => {
                formatter.write_str("GPUI window has no native window handle")
            }
            Self::UnexpectedWindowHandle => {
                formatter.write_str("GPUI window handle is not a Win32 HWND")
            }
            Self::InvalidSize => formatter.write_str("native window size must be positive"),
            Self::Platform { operation, code } => {
                write!(formatter, "{operation} failed with Win32 code {code}")
            }
        }
    }
}

impl std::error::Error for NativeError {}

/// OS operations required by floating native windows.
///
/// Coordinates and sizes use physical screen pixels. Core and GPUI logical
/// coordinates must be converted by the caller using scale_factor and
/// client_origin. This crate does not know about DockCore.
pub trait NativeWindowOps {
    fn hwnd_of(&self, window: &Window) -> Result<NativeWindowId, NativeError>;

    fn cursor_position(&self) -> Result<PhysicalPoint, NativeError>;
    fn left_button_down(&self) -> Result<bool, NativeError>;
    fn key_down(&self, key: NativeKey) -> Result<bool, NativeError>;

    fn window_rect(&self, window: NativeWindowId) -> Result<PhysicalRect, NativeError>;
    fn client_origin(&self, window: NativeWindowId) -> Result<PhysicalPoint, NativeError>;
    /// Returns client-content dimensions in physical pixels, excluding the non-client frame.
    fn client_size(&self, window: NativeWindowId) -> Result<PhysicalSize, NativeError>;
    fn scale_factor(&self, window: NativeWindowId) -> Result<f32, NativeError>;
    fn move_window(
        &self,
        window: NativeWindowId,
        position: PhysicalPoint,
    ) -> Result<(), NativeError>;
    /// Sets the requested client-content size in physical pixels.
    /// The native implementation accounts for the non-client window frame.
    fn resize_window(
        &self,
        window: NativeWindowId,
        size: PhysicalSize,
    ) -> Result<(), NativeError>;

    /// Atomically sets the outer window origin and requested client-content size.
    /// Platforms should update both in one native call to avoid visible intermediate movement.
    fn set_window_bounds(
        &self,
        window: NativeWindowId,
        position: PhysicalPoint,
        size: PhysicalSize,
    ) -> Result<(), NativeError> {
        self.move_window(window, position)?;
        self.resize_window(window, size)
    }

    fn show(&self, window: NativeWindowId) -> Result<(), NativeError>;
    fn show_no_activate(&self, window: NativeWindowId) -> Result<(), NativeError>;
    fn activate(&self, window: NativeWindowId) -> Result<(), NativeError>;
    /// Returns the current OS foreground window, or None when no window is foreground.
    fn foreground_window(&self) -> Result<Option<NativeWindowId>, NativeError>;
    fn hide(&self, window: NativeWindowId) -> Result<(), NativeError>;

    /// Assigns the native owner of a top-level floating window.
    ///
    /// On Windows this is an application-lifecycle relationship: the owned window
    /// is hidden when its owner is minimized and destroyed with its owner. It must
    /// not be used to represent whether a panel is docked or floating.
    fn set_owner(
        &self,
        window: NativeWindowId,
        owner: NativeWindowId,
    ) -> Result<(), NativeError>;

    fn enable_resize(&self, window: NativeWindowId) -> Result<(), NativeError>;
    fn disable_resize(&self, window: NativeWindowId) -> Result<(), NativeError>;
    /// Reports whether this HWND is inside the OS modal move/resize loop.
    fn is_resizing(&self, window: NativeWindowId) -> Result<bool, NativeError>;
    /// Sets the minimum client-content size in physical pixels. The native
    /// implementation includes the current non-client frame in WM_GETMINMAXINFO.
    /// Pass None to remove the dynamic native constraint.
    fn set_min_size(
        &self,
        window: NativeWindowId,
        size: Option<PhysicalSize>,
    ) -> Result<(), NativeError>;

    /// Finds the topmost window under a screen point. Foreign windows are
    /// reported separately so callers cannot mistake them for empty desktop space.
    /// The dragged window can be excluded so it cannot become its own drop target.
    fn window_from_point(
        &self,
        point: PhysicalPoint,
        known_windows: &[NativeWindowId],
        exclude: Option<NativeWindowId>,
    ) -> Result<NativeWindowAtPoint, NativeError>;

    fn monitor_work_areas(&self) -> Result<Vec<PhysicalRect>, NativeError>;

    /// Pair this request with end_high_resolution_timer when polling ends.
    fn begin_high_resolution_timer(&self) -> Result<(), NativeError>;
    fn end_high_resolution_timer(&self) -> Result<(), NativeError>;
}

/// Resolves the first visible top-level window under a screen point.
///
/// The callback yields windows lazily in native Z-order, avoiding a per-pointer
/// allocation while the OS drag loop is polling. The dragged HWND is transparent
/// to docking hit-testing; any other unknown visible window blocks the drop.
fn topmost_known_window_under_point(
    point: PhysicalPoint,
    known_windows: &[NativeWindowId],
    exclude: Option<NativeWindowId>,
    mut next_window: impl FnMut() -> Option<(NativeWindowId, PhysicalRect)>,
) -> NativeWindowAtPoint {
    while let Some((window, rect)) = next_window() {
        if Some(window) == exclude || !rect.contains(point) {
            continue;
        }
        return if known_windows.contains(&window) {
            NativeWindowAtPoint::Known(window)
        } else {
            NativeWindowAtPoint::Foreign
        };
    }
    NativeWindowAtPoint::Empty
}

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::mem::size_of;
    use windows::core::BOOL;
    use windows::Win32::{
        Foundation::{
            GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError, WIN32_ERROR, WPARAM,
        },
        Graphics::Gdi::{
            ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
        },
        Media::{timeBeginPeriod, timeEndPeriod},
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON},
            WindowsAndMessaging::*,
        },
    };

    const RESIZE_REDRAW_TIMER_ID: usize = 2;
    const RESIZE_REDRAW_INTERVAL_MS: u32 = 8;
    const WM_GPUI_FORCE_UPDATE_WINDOW: u32 = WM_USER + 5;
    const MIN_SIZE_SUBCLASS_ID: usize = 0x444f_434b;
    const RESIZE_LOOP_SUBCLASS_ID: usize = 0x5253_495a;

    #[derive(Clone, Copy, Default)]
    pub struct SystemNativeWindowOps;

    fn hwnd(window: NativeWindowId) -> HWND {
        HWND(window.as_raw() as _)
    }

    /// Removes the window from the taskbar (and Alt+Tab).
    ///
    /// Clears `WS_EX_APPWINDOW` and sets `WS_EX_TOOLWINDOW`, which is what GPUI
    /// itself does for `WindowKind::PopUp` on Windows. `WindowKind::Floating`
    /// is handled like a normal window, so it would otherwise get a taskbar button.
    ///
    /// Call this while the window is still hidden: the style is applied on the
    /// next show, so the taskbar button never appears.
    pub(super) fn hide_from_taskbar(window: NativeWindowId) -> bool {
        // SAFETY: The HWND originates from GPUI; callers must not use it after its window closes.
        unsafe {
            let native = hwnd(window);
            let ex_style = GetWindowLongPtrW(native, GWL_EXSTYLE) as u32;
            let ex_style = (ex_style & !WS_EX_APPWINDOW.0) | WS_EX_TOOLWINDOW.0;
            let _previous = SetWindowLongPtrW(native, GWL_EXSTYLE, ex_style as isize);

            // Make Windows re-read the extended style.
            SetWindowPos(
                native,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            )
        }
        .is_ok()
    }

    fn platform_error(operation: &'static str, error: windows::core::Error) -> NativeError {
        NativeError::Platform {
            operation,
            code: error.code().0 as u32,
        }
    }

    fn last_error(operation: &'static str) -> NativeError {
        // SAFETY: GetLastError reads the calling thread's Win32 error state.
        let code = unsafe { GetLastError().0 };
        NativeError::Platform { operation, code }
    }

    /// Removes the resizable frame after a failed attempt to install native
    /// resize tracking. Cleanup is best-effort; preserve the original error.
    fn restore_non_resizable_frame(window: HWND) {
        // SAFETY: The HWND is the live GPUI-owned window passed to enable_resize.
        unsafe {
            let style = GetWindowLongPtrW(window, GWL_STYLE) as u32 & !WS_THICKFRAME.0;
            let _ = SetWindowLongPtrW(window, GWL_STYLE, style as isize);
            let _ = SetWindowPos(
                window,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE
                    | SWP_NOSIZE
                    | SWP_NOZORDER
                    | SWP_NOACTIVATE
                    | SWP_FRAMECHANGED,
            );
        }
    }

    fn rect_from_native(rect: RECT) -> PhysicalRect {
        PhysicalRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    fn left_button_is_down() -> bool {
        // SAFETY: GetAsyncKeyState has no pointer arguments.
        unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16) & 0x8000 != 0 }
    }

    struct ResizeLoopState {
        active: bool,
    }

    unsafe extern "system" fn resize_loop_subclass_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass_id: usize,
        reference_data: usize,
    ) -> LRESULT {
        if reference_data == 0 {
            // SAFETY: Forward the message because no state was attached.
            return unsafe { DefSubclassProc(window, message, wparam, lparam) };
        }

        let state = reference_data as *mut ResizeLoopState;
        if message == WM_ENTERSIZEMOVE {
            // SAFETY: The subclass owns this live state allocation.
            unsafe { (*state).active = true };
        }

        // SAFETY: Forward the message to GPUI/the remaining subclass chain.
        let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };

        if message == WM_EXITSIZEMOVE {
            // SAFETY: The subclass owns this live state allocation.
            unsafe { (*state).active = false };
        }
        if message == WM_NCDESTROY {
            // SAFETY: Detach this callback before freeing its owned state.
            unsafe {
                let _ = RemoveWindowSubclass(window, Some(resize_loop_subclass_proc), subclass_id);
                drop(Box::from_raw(state));
            }
        }

        result
    }

    unsafe extern "system" fn resize_redraw_timer_proc(
        window: HWND,
        _message: u32,
        _timer_id: usize,
        _time: u32,
    ) {
        let mut reference_data = 0usize;
        // SAFETY: Query the resize-loop subclass attached to this same HWND.
        let tracking_installed = unsafe {
            GetWindowSubclass(
                window,
                Some(resize_loop_subclass_proc),
                RESIZE_LOOP_SUBCLASS_ID,
                Some(&mut reference_data as *mut usize),
            )
        }
        .as_bool();
        let resizing = tracking_installed
            && reference_data != 0
            // SAFETY: The subclass retains this state until it is removed or
            // WM_NCDESTROY runs on the same UI thread.
            && unsafe { (*(reference_data as *const ResizeLoopState)).active };

        if resizing {
            // SAFETY: window is the HWND supplied by SetTimer. This message ID
            // is handled by the pinned GPUI Windows backend.
            let _ = unsafe {
                PostMessageW(
                    Some(window),
                    WM_GPUI_FORCE_UPDATE_WINDOW,
                    WPARAM(0),
                    LPARAM(0),
                )
            };
        }
    }

    #[derive(Clone, Copy)]
    struct MinTrackSize {
        width: i32,
        height: i32,
    }

    unsafe extern "system" fn min_track_size_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        subclass_id: usize,
        reference_data: usize,
    ) -> LRESULT {
        // SAFETY: The callback and reference data are registered together by
        // SetWindowSubclass; the Box remains live until WM_NCDESTROY/removal.
        let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };

        if message == WM_GETMINMAXINFO && lparam.0 != 0 && reference_data != 0 {
            // SAFETY: WM_GETMINMAXINFO supplies a writable MINMAXINFO pointer;
            // reference_data points to the still-live MinTrackSize allocation.
            unsafe {
                let limits = &*(reference_data as *const MinTrackSize);
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize.x = limits.width;
                info.ptMinTrackSize.y = limits.height;
            }
        }

        if message == WM_NCDESTROY && reference_data != 0 {
            // SAFETY: Remove this exact subclass and free its Box exactly once.
            unsafe {
                let _ = RemoveWindowSubclass(window, Some(min_track_size_proc), subclass_id);
                drop(Box::from_raw(reference_data as *mut MinTrackSize));
            }
        }

        result
    }

    struct MonitorCollection {
        work_areas: Vec<PhysicalRect>,
        error: Option<NativeError>,
    }

    unsafe extern "system" fn monitor_enum_proc(
        monitor: HMONITOR,
        _dc: HDC,
        _monitor_rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: EnumDisplayMonitors calls this synchronously with a pointer
        // to the live MonitorCollection passed as LPARAM.
        let collection = unsafe { &mut *(data.0 as *mut MonitorCollection) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };

        // SAFETY: monitor is the HMONITOR supplied by EnumDisplayMonitors and
        // info is a valid writable MONITORINFO.
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            collection.error = Some(last_error("GetMonitorInfoW"));
            return BOOL(0);
        }

        collection.work_areas.push(rect_from_native(info.rcWork));
        BOOL(1)
    }

    impl NativeWindowOps for SystemNativeWindowOps {
        fn hwnd_of(&self, window: &Window) -> Result<NativeWindowId, NativeError> {
            let handle = HasWindowHandle::window_handle(window)
                .map_err(|_| NativeError::WindowHandleUnavailable)?;

            match handle.as_raw() {
                RawWindowHandle::Win32(handle) => NativeWindowId::from_raw(handle.hwnd.get())
                    .ok_or(NativeError::WindowHandleUnavailable),
                _ => Err(NativeError::UnexpectedWindowHandle),
            }
        }

        fn cursor_position(&self) -> Result<PhysicalPoint, NativeError> {
            let mut point = POINT::default();
            // SAFETY: point is a valid writable POINT.
            unsafe { GetCursorPos(&mut point) }
                .map_err(|error| platform_error("GetCursorPos", error))?;
            Ok(PhysicalPoint {
                x: point.x,
                y: point.y,
            })
        }

        fn left_button_down(&self) -> Result<bool, NativeError> {
            Ok(left_button_is_down())
        }

        fn key_down(&self, key: NativeKey) -> Result<bool, NativeError> {
            let virtual_key = match key {
                NativeKey::Escape => VK_ESCAPE,
            };
            // SAFETY: GetAsyncKeyState accepts a virtual-key code and has no
            // pointer arguments.
            Ok(unsafe { (GetAsyncKeyState(virtual_key.0 as i32) as u16) & 0x8000 != 0 })
        }

        fn window_rect(&self, window: NativeWindowId) -> Result<PhysicalRect, NativeError> {
            let mut rect = RECT::default();
            // SAFETY: window is an HWND obtained from a live GPUI Window and
            // rect is writable.
            unsafe { GetWindowRect(hwnd(window), &mut rect) }
                .map_err(|error| platform_error("GetWindowRect", error))?;
            Ok(rect_from_native(rect))
        }

        fn client_origin(&self, window: NativeWindowId) -> Result<PhysicalPoint, NativeError> {
            let mut point = POINT::default();
            // SAFETY: point is writable and window is a GPUI-owned HWND.
            unsafe { ClientToScreen(hwnd(window), &mut point) }
                .ok()
                .map_err(|error| platform_error("ClientToScreen", error))?;
            Ok(PhysicalPoint {
                x: point.x,
                y: point.y,
            })
        }

        fn client_size(&self, window: NativeWindowId) -> Result<PhysicalSize, NativeError> {
            let mut rect = RECT::default();
            // SAFETY: rect is a writable output for this live HWND.
            unsafe { GetClientRect(hwnd(window), &mut rect) }
                .map_err(|error| platform_error("GetClientRect", error))?;
            let width = rect.right.checked_sub(rect.left).ok_or(NativeError::InvalidSize)?;
            let height = rect.bottom.checked_sub(rect.top).ok_or(NativeError::InvalidSize)?;
            let size = PhysicalSize { width, height };
            if !size.is_valid() {
                return Err(NativeError::InvalidSize);
            }
            Ok(size)
        }

        fn scale_factor(&self, window: NativeWindowId) -> Result<f32, NativeError> {
            // SAFETY: GetDpiForWindow accepts an HWND and returns zero if it
            // is not associated with a valid window.
            let dpi = unsafe { GetDpiForWindow(hwnd(window)) };
            if dpi == 0 {
                return Err(last_error("GetDpiForWindow"));
            }
            Ok(dpi as f32 / 96.0)
        }

        fn move_window(
            &self,
            window: NativeWindowId,
            position: PhysicalPoint,
        ) -> Result<(), NativeError> {
            // SAFETY: Move only bounds; preserve Z-order and do not activate.
            unsafe {
                SetWindowPos(
                    hwnd(window),
                    None,
                    position.x,
                    position.y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .map_err(|error| platform_error("SetWindowPos(move)", error))
        }

        fn resize_window(
            &self,
            window: NativeWindowId,
            size: PhysicalSize,
        ) -> Result<(), NativeError> {
            if !size.is_valid() {
                return Err(NativeError::InvalidSize);
            }

            let native = hwnd(window);
            let mut window_rect = RECT::default();
            let mut client_rect = RECT::default();
            // SAFETY: Both RECT values are writable outputs for this live HWND.
            unsafe {
                GetWindowRect(native, &mut window_rect)
                    .map_err(|error| platform_error("GetWindowRect(resize)", error))?;
                GetClientRect(native, &mut client_rect)
                    .map_err(|error| platform_error("GetClientRect(resize)", error))?;
            }

            let window_width = window_rect.right.checked_sub(window_rect.left)
                .ok_or(NativeError::InvalidSize)?;
            let window_height = window_rect.bottom.checked_sub(window_rect.top)
                .ok_or(NativeError::InvalidSize)?;
            let client_width = client_rect.right.checked_sub(client_rect.left)
                .ok_or(NativeError::InvalidSize)?;
            let client_height = client_rect.bottom.checked_sub(client_rect.top)
                .ok_or(NativeError::InvalidSize)?;
            let frame_width = window_width.saturating_sub(client_width).max(0);
            let frame_height = window_height.saturating_sub(client_height).max(0);
            let outer_width = size.width.checked_add(frame_width)
                .ok_or(NativeError::InvalidSize)?;
            let outer_height = size.height.checked_add(frame_height)
                .ok_or(NativeError::InvalidSize)?;

            // Core sizes describe the client area. SetWindowPos expects the
            // complete outer HWND size, so preserve the current non-client frame.
            // SAFETY: Resize only; preserve position, Z-order and activation.
            unsafe {
                SetWindowPos(
                    native,
                    None,
                    0,
                    0,
                    outer_width,
                    outer_height,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .map_err(|error| platform_error("SetWindowPos(resize)", error))
        }

        fn set_window_bounds(
            &self,
            window: NativeWindowId,
            position: PhysicalPoint,
            size: PhysicalSize,
        ) -> Result<(), NativeError> {
            if !size.is_valid() {
                return Err(NativeError::InvalidSize);
            }

            let native = hwnd(window);
            let mut window_rect = RECT::default();
            let mut client_rect = RECT::default();
            // SAFETY: Both RECT values are writable outputs for this live HWND.
            unsafe {
                GetWindowRect(native, &mut window_rect)
                    .map_err(|error| platform_error("GetWindowRect(bounds)", error))?;
                GetClientRect(native, &mut client_rect)
                    .map_err(|error| platform_error("GetClientRect(bounds)", error))?;
            }

            let window_width = window_rect.right.checked_sub(window_rect.left)
                .ok_or(NativeError::InvalidSize)?;
            let window_height = window_rect.bottom.checked_sub(window_rect.top)
                .ok_or(NativeError::InvalidSize)?;
            let client_width = client_rect.right.checked_sub(client_rect.left)
                .ok_or(NativeError::InvalidSize)?;
            let client_height = client_rect.bottom.checked_sub(client_rect.top)
                .ok_or(NativeError::InvalidSize)?;
            let frame_width = window_width.saturating_sub(client_width).max(0);
            let frame_height = window_height.saturating_sub(client_height).max(0);
            let outer_width = size.width.checked_add(frame_width)
                .ok_or(NativeError::InvalidSize)?;
            let outer_height = size.height.checked_add(frame_height)
                .ok_or(NativeError::InvalidSize)?;

            // Set position and full outer size together. The requested size is
            // client-content size; preserve the existing non-client frame.
            // SAFETY: One SetWindowPos updates both edges while preserving Z-order and activation.
            unsafe {
                SetWindowPos(
                    native,
                    None,
                    position.x,
                    position.y,
                    outer_width,
                    outer_height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .map_err(|error| platform_error("SetWindowPos(bounds)", error))
        }

        fn show(&self, window: NativeWindowId) -> Result<(), NativeError> {
            // SAFETY: window is a GPUI-owned HWND.
            let _ = unsafe { ShowWindowAsync(hwnd(window), SW_SHOW) };
            Ok(())
        }

        fn show_no_activate(&self, window: NativeWindowId) -> Result<(), NativeError> {
            // SAFETY: window is a GPUI-owned HWND.
            let _ = unsafe { ShowWindowAsync(hwnd(window), SW_SHOWNOACTIVATE) };
            Ok(())
        }

        fn activate(&self, window: NativeWindowId) -> Result<(), NativeError> {
            let native = hwnd(window);
            // SAFETY: IsIconic and ShowWindowAsync operate on the supplied HWND.
            unsafe {
                if IsIconic(native).as_bool() {
                    let _ = ShowWindowAsync(native, SW_RESTORE);
                }
            }
            // SAFETY: SetForegroundWindow is called for a window owned by this
            // process. Windows can still deny activation due to foreground policy.
            if !unsafe { SetForegroundWindow(native) }.as_bool() {
                return Err(last_error("SetForegroundWindow"));
            }
            Ok(())
        }

        fn foreground_window(&self) -> Result<Option<NativeWindowId>, NativeError> {
            // SAFETY: GetForegroundWindow has no pointer arguments and may return a null HWND.
            let foreground = unsafe { GetForegroundWindow() };
            Ok(NativeWindowId::from_raw(foreground.0 as isize))
        }

        fn hide(&self, window: NativeWindowId) -> Result<(), NativeError> {
            // SAFETY: window is a GPUI-owned HWND.
            let _ = unsafe { ShowWindowAsync(hwnd(window), SW_HIDE) };
            Ok(())
        }

        fn set_owner(
            &self,
            window: NativeWindowId,
            owner: NativeWindowId,
        ) -> Result<(), NativeError> {
            // SAFETY: Both IDs originate from GPUI-owned top-level windows.
            // Clear the thread error first because a successful call may return
            // zero when the previous owner was null.
            unsafe {
                SetLastError(WIN32_ERROR(0));
            }
            let previous_owner =
                unsafe { SetWindowLongPtrW(hwnd(window), GWLP_HWNDPARENT, owner.as_raw()) };
            if previous_owner == 0 {
                let error = last_error("SetWindowLongPtrW(owner)");
                if matches!(&error, NativeError::Platform { code, .. } if *code != 0) {
                    return Err(error);
                }
            }
            Ok(())
        }

        fn enable_resize(&self, window: NativeWindowId) -> Result<(), NativeError> {
            let native = hwnd(window);
            // SAFETY: The HWND belongs to GPUI. Add the resizable frame and
            // apply the non-client update without moving or activating it.
            let style = unsafe { GetWindowLongPtrW(native, GWL_STYLE) as u32 | WS_THICKFRAME.0 };
            unsafe {
                let _ = SetWindowLongPtrW(native, GWL_STYLE, style as isize);
            }
            if let Err(error) = unsafe {
                SetWindowPos(
                    native,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE
                        | SWP_NOSIZE
                        | SWP_NOZORDER
                        | SWP_NOACTIVATE
                        | SWP_FRAMECHANGED,
                )
            } {
                let error = platform_error("SetWindowPos(enable resize)", error);
                restore_non_resizable_frame(native);
                return Err(error);
            }

            let mut reference_data = 0usize;
            // SAFETY: Query whether the resize-loop state is already attached.
            let mut tracking_installed = unsafe {
                GetWindowSubclass(
                    native,
                    Some(resize_loop_subclass_proc),
                    RESIZE_LOOP_SUBCLASS_ID,
                    Some(&mut reference_data as *mut usize),
                )
            }
            .as_bool();
            if !tracking_installed {
                let pointer = Box::into_raw(Box::new(ResizeLoopState { active: false }));
                // SAFETY: The window subclass owns this allocation until it is
                // explicitly removed or WM_NCDESTROY is received.
                if !unsafe {
                    SetWindowSubclass(
                        native,
                        Some(resize_loop_subclass_proc),
                        RESIZE_LOOP_SUBCLASS_ID,
                        pointer as usize,
                    )
                }
                .as_bool()
                {
                    let error = last_error("SetWindowSubclass(resize loop)");
                    // SAFETY: Registration failed, so the callback did not
                    // retain the allocation.
                    unsafe { drop(Box::from_raw(pointer)) };
                    restore_non_resizable_frame(native);
                    return Err(error);
                }
                reference_data = pointer as usize;
                tracking_installed = true;
            }

            // SAFETY: Attach an 8 ms UI-thread timer to this GPUI window.
            let timer = unsafe {
                SetTimer(
                    Some(native),
                    RESIZE_REDRAW_TIMER_ID,
                    RESIZE_REDRAW_INTERVAL_MS,
                    Some(resize_redraw_timer_proc),
                )
            };
            if timer == 0 {
                let error = last_error("SetTimer(resize redraw)");
                // SAFETY: A failed SetTimer may follow successful subclass
                // installation. Remove the timer defensively, then detach and
                // release the subclass state only if detaching succeeds.
                unsafe {
                    let _ = KillTimer(Some(native), RESIZE_REDRAW_TIMER_ID);
                }
                if tracking_installed
                    && unsafe {
                        RemoveWindowSubclass(
                            native,
                            Some(resize_loop_subclass_proc),
                            RESIZE_LOOP_SUBCLASS_ID,
                        )
                    }
                    .as_bool()
                    && reference_data != 0
                {
                    // SAFETY: The subclass was detached, so it no longer owns
                    // or accesses this state allocation.
                    unsafe { drop(Box::from_raw(reference_data as *mut ResizeLoopState)) };
                }
                restore_non_resizable_frame(native);
                return Err(error);
            }
            Ok(())
        }

        fn disable_resize(&self, window: NativeWindowId) -> Result<(), NativeError> {
            let native = hwnd(window);
            // SAFETY: Stop the timer before detaching its resize-loop state.
            unsafe { let _ = KillTimer(Some(native), RESIZE_REDRAW_TIMER_ID); }

            let mut reference_data = 0usize;
            // SAFETY: Query the exact subclass callback and ID installed above.
            let tracking_installed = unsafe {
                GetWindowSubclass(
                    native,
                    Some(resize_loop_subclass_proc),
                    RESIZE_LOOP_SUBCLASS_ID,
                    Some(&mut reference_data as *mut usize),
                )
            }
            .as_bool();
            if tracking_installed {
                // SAFETY: Detach the callback before freeing its retained state.
                if !unsafe {
                    RemoveWindowSubclass(native, Some(resize_loop_subclass_proc), RESIZE_LOOP_SUBCLASS_ID)
                }
                .as_bool()
                {
                    return Err(last_error("RemoveWindowSubclass(resize loop)"));
                }
                if reference_data != 0 {
                    // SAFETY: The callback is detached and can no longer access
                    // this allocation.
                    unsafe { drop(Box::from_raw(reference_data as *mut ResizeLoopState)) };
                }
            }

            // SAFETY: Remove WS_THICKFRAME from the same GPUI-owned HWND and
            // apply the non-client frame update without moving or activating it.
            unsafe {
                let style = GetWindowLongPtrW(native, GWL_STYLE) as u32 & !WS_THICKFRAME.0;
                let _ = SetWindowLongPtrW(native, GWL_STYLE, style as isize);
                SetWindowPos(
                    native,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE
                        | SWP_NOSIZE
                        | SWP_NOZORDER
                        | SWP_NOACTIVATE
                        | SWP_FRAMECHANGED,
                )
            }
            .map_err(|error| platform_error("SetWindowPos(disable resize)", error))
        }

        fn is_resizing(&self, window: NativeWindowId) -> Result<bool, NativeError> {
            let mut reference_data = 0usize;
            // SAFETY: Query the subclass and reference data attached to this HWND.
            let installed = unsafe {
                GetWindowSubclass(
                    hwnd(window),
                    Some(resize_loop_subclass_proc),
                    RESIZE_LOOP_SUBCLASS_ID,
                    Some(&mut reference_data as *mut usize),
                )
            }
            .as_bool();
            if !installed || reference_data == 0 {
                return Ok(false);
            }

            // SAFETY: The registered subclass owns this state allocation until
            // it is detached or WM_NCDESTROY runs, both on the UI thread.
            Ok(unsafe { (*(reference_data as *const ResizeLoopState)).active })
        }

        fn set_min_size(
            &self,
            window: NativeWindowId,
            size: Option<PhysicalSize>,
        ) -> Result<(), NativeError> {
            if let Some(size) = size
                && !size.is_valid()
            {
                return Err(NativeError::InvalidSize);
            }

            let native = hwnd(window);
            let outer_size = if let Some(client_size) = size {
                let mut window_rect = RECT::default();
                let mut client_rect = RECT::default();
                // SAFETY: Both RECT values are writable outputs for this live HWND.
                unsafe {
                    GetWindowRect(native, &mut window_rect)
                        .map_err(|error| platform_error("GetWindowRect(min size)", error))?;
                    GetClientRect(native, &mut client_rect)
                        .map_err(|error| platform_error("GetClientRect(min size)", error))?;
                }
                let window_width = window_rect.right.checked_sub(window_rect.left)
                    .ok_or(NativeError::InvalidSize)?;
                let window_height = window_rect.bottom.checked_sub(window_rect.top)
                    .ok_or(NativeError::InvalidSize)?;
                let client_width = client_rect.right.checked_sub(client_rect.left)
                    .ok_or(NativeError::InvalidSize)?;
                let client_height = client_rect.bottom.checked_sub(client_rect.top)
                    .ok_or(NativeError::InvalidSize)?;
                let frame_width = window_width.saturating_sub(client_width).max(0);
                let frame_height = window_height.saturating_sub(client_height).max(0);
                Some(PhysicalSize {
                    width: client_size.width.checked_add(frame_width)
                        .ok_or(NativeError::InvalidSize)?,
                    height: client_size.height.checked_add(frame_height)
                        .ok_or(NativeError::InvalidSize)?,
                })
            } else {
                None
            };
            let mut reference_data = 0usize;

            // SAFETY: Query the exact subclass callback and ID used below.
            let already_installed = unsafe {
                GetWindowSubclass(
                    native,
                    Some(min_track_size_proc),
                    MIN_SIZE_SUBCLASS_ID,
                    Some(&mut reference_data as *mut usize),
                )
            }
            .as_bool();

            match outer_size {
                Some(size) if already_installed => {
                    // SAFETY: reference_data is the live allocation retained
                    // by this window's min-size subclass.
                    unsafe {
                        *(reference_data as *mut MinTrackSize) = MinTrackSize {
                            width: size.width,
                            height: size.height,
                        };
                    }
                    Ok(())
                }
                Some(size) => {
                    let pointer = Box::into_raw(Box::new(MinTrackSize {
                        width: size.width,
                        height: size.height,
                    }));
                    // SAFETY: The subclass owns this pointer until WM_NCDESTROY
                    // or explicit removal.
                    if !unsafe {
                        SetWindowSubclass(
                            native,
                            Some(min_track_size_proc),
                            MIN_SIZE_SUBCLASS_ID,
                            pointer as usize,
                        )
                    }
                    .as_bool()
                    {
                        let error = last_error("SetWindowSubclass(min size)");
                        // SAFETY: Registration failed, so the callback did not
                        // retain pointer and it is still owned here.
                        unsafe { drop(Box::from_raw(pointer)) };
                        return Err(error);
                    }
                    Ok(())
                }
                None if already_installed => {
                    // SAFETY: Remove the registered callback before freeing
                    // the reference data it owns.
                    if !unsafe {
                        RemoveWindowSubclass(
                            native,
                            Some(min_track_size_proc),
                            MIN_SIZE_SUBCLASS_ID,
                        )
                    }
                    .as_bool()
                    {
                        return Err(last_error("RemoveWindowSubclass(min size)"));
                    }
                    // SAFETY: The callback was successfully detached, so it
                    // can no longer access the allocation.
                    unsafe { drop(Box::from_raw(reference_data as *mut MinTrackSize)) };
                    Ok(())
                }
                None => Ok(()),
            }
        }

        fn window_from_point(
            &self,
            point: PhysicalPoint,
            known_windows: &[NativeWindowId],
            exclude: Option<NativeWindowId>,
        ) -> Result<NativeWindowAtPoint, NativeError> {
            // SAFETY: Walk the desktop's top-level Z-order synchronously.
            let mut current = unsafe { GetTopWindow(None) }.ok();
            let target = topmost_known_window_under_point(
                point,
                known_windows,
                exclude,
                || {
                    while let Some(native) = current {
                        // SAFETY: Advance to the next top-level HWND in Z-order.
                        current = unsafe { GetWindow(native, GW_HWNDNEXT) }.ok();
                        let Some(id) = NativeWindowId::from_raw(native.0 as isize) else {
                            continue;
                        };
                        // SAFETY: native is the current top-level HWND.
                        let visible = unsafe {
                            IsWindowVisible(native).as_bool() && !IsIconic(native).as_bool()
                        };
                        if !visible {
                            continue;
                        }
                        let mut rect = RECT::default();
                        // SAFETY: rect is a valid writable RECT.
                        if unsafe { GetWindowRect(native, &mut rect) }.is_ok() {
                            return Some((id, rect_from_native(rect)));
                        }
                    }
                    None
                },
            );
            Ok(target)
        }

        fn monitor_work_areas(&self) -> Result<Vec<PhysicalRect>, NativeError> {
            let mut collection = MonitorCollection {
                work_areas: Vec::new(),
                error: None,
            };
            // SAFETY: The LPARAM points to the live collection and the callback
            // is invoked synchronously by EnumDisplayMonitors.
            let result = unsafe {
                EnumDisplayMonitors(
                    None,
                    None,
                    Some(monitor_enum_proc),
                    LPARAM((&mut collection as *mut MonitorCollection) as isize),
                )
            };

            if let Some(error) = collection.error {
                return Err(error);
            }
            if !result.as_bool() {
                return Err(last_error("EnumDisplayMonitors"));
            }
            Ok(collection.work_areas)
        }

        fn begin_high_resolution_timer(&self) -> Result<(), NativeError> {
            // SAFETY: timeBeginPeriod has no pointer arguments. The caller must
            // pair this request with one end_high_resolution_timer call.
            let result = unsafe { timeBeginPeriod(1) };
            if result == 0 {
                Ok(())
            } else {
                Err(NativeError::Platform {
                    operation: "timeBeginPeriod",
                    code: result,
                })
            }
        }

        fn end_high_resolution_timer(&self) -> Result<(), NativeError> {
            // SAFETY: Release the 1 ms resolution requested above.
            let result = unsafe { timeEndPeriod(1) };
            if result == 0 {
                Ok(())
            } else {
                Err(NativeError::Platform {
                    operation: "timeEndPeriod",
                    code: result,
                })
            }
        }
    }
}

#[cfg(not(windows))]
mod stub_impl {
    use super::*;

    #[derive(Clone, Copy, Default)]
    pub struct SystemNativeWindowOps;

    impl NativeWindowOps for SystemNativeWindowOps {
        fn hwnd_of(&self, _: &Window) -> Result<NativeWindowId, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn cursor_position(&self) -> Result<PhysicalPoint, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn left_button_down(&self) -> Result<bool, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn key_down(&self, _: NativeKey) -> Result<bool, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn window_rect(&self, _: NativeWindowId) -> Result<PhysicalRect, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn client_origin(&self, _: NativeWindowId) -> Result<PhysicalPoint, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn client_size(&self, _: NativeWindowId) -> Result<PhysicalSize, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn scale_factor(&self, _: NativeWindowId) -> Result<f32, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn move_window(
            &self,
            _: NativeWindowId,
            _: PhysicalPoint,
        ) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn resize_window(
            &self,
            _: NativeWindowId,
            _: PhysicalSize,
        ) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn show(&self, _: NativeWindowId) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn show_no_activate(&self, _: NativeWindowId) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn activate(&self, _: NativeWindowId) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn foreground_window(&self) -> Result<Option<NativeWindowId>, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn hide(&self, _: NativeWindowId) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn set_owner(
            &self,
            _: NativeWindowId,
            _: NativeWindowId,
        ) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn enable_resize(&self, _: NativeWindowId) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn disable_resize(&self, _: NativeWindowId) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn is_resizing(&self, _: NativeWindowId) -> Result<bool, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn set_min_size(
            &self,
            _: NativeWindowId,
            _: Option<PhysicalSize>,
        ) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn window_from_point(
            &self,
            _: PhysicalPoint,
            _: &[NativeWindowId],
            _: Option<NativeWindowId>,
        ) -> Result<NativeWindowAtPoint, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn monitor_work_areas(&self) -> Result<Vec<PhysicalRect>, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn begin_high_resolution_timer(&self) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
        fn end_high_resolution_timer(&self) -> Result<(), NativeError> {
            Err(NativeError::Unsupported)
        }
    }
}

#[cfg(windows)]
pub use windows_impl::SystemNativeWindowOps;
#[cfg(not(windows))]
pub use stub_impl::SystemNativeWindowOps;

/// Removes the taskbar and Alt+Tab entry from a hidden floating window.
#[cfg(windows)]
pub fn hide_from_taskbar(window: NativeWindowId) -> bool {
    windows_impl::hide_from_taskbar(window)
}

/// Taskbar styling is only available for native Windows windows.
#[cfg(not(windows))]
pub fn hide_from_taskbar(_: NativeWindowId) -> bool {
    false
}


#[cfg(test)]
mod drop_window_target_tests {
    use super::{NativeWindowAtPoint, NativeWindowId, PhysicalPoint, PhysicalRect, topmost_known_window_under_point};

    fn window(id: isize) -> NativeWindowId {
        NativeWindowId::from_raw(id).unwrap()
    }

    fn bounds() -> PhysicalRect {
        PhysicalRect {
            left: 100,
            top: 100,
            right: 500,
            bottom: 400,
        }
    }

    fn resolve(
        point: PhysicalPoint,
        windows: &[(NativeWindowId, PhysicalRect)],
        known: &[NativeWindowId],
        exclude: Option<NativeWindowId>,
    ) -> NativeWindowAtPoint {
        let mut index = 0;
        topmost_known_window_under_point(point, known, exclude, || {
            let result = windows.get(index).copied();
            if result.is_some() {
                index += 1;
            }
            result
        })
    }

    #[test]
    fn empty_desktop_point_is_distinct_from_a_foreign_window() {
        let point = PhysicalPoint { x: 700, y: 500 };
        assert_eq!(
            resolve(point, &[], &[], None),
            NativeWindowAtPoint::Empty,
        );
    }

    #[test]
    fn unrelated_topmost_window_blocks_a_dock_window_behind_it() {
        let point = PhysicalPoint { x: 200, y: 200 };
        let foreign_window = window(1);
        let main_window = window(2);
        let windows = [
            (foreign_window, bounds()),
            (main_window, bounds()),
        ];

        assert_eq!(resolve(point, &windows, &[main_window], None), NativeWindowAtPoint::Foreign);
    }

    #[test]
    fn dragged_source_window_is_skipped_so_target_behind_it_can_receive_drop() {
        let point = PhysicalPoint { x: 200, y: 200 };
        let source_window = window(1);
        let target_window = window(2);
        let windows = [
            (source_window, bounds()),
            (target_window, bounds()),
        ];

        assert_eq!(
            resolve(
                point,
                &windows,
                &[source_window, target_window],
                Some(source_window),
            ),
            NativeWindowAtPoint::Known(target_window),
        );
    }

    #[test]
    fn drag_host_overlay_is_skipped_so_a_dock_window_behind_it_can_receive_drop() {
        let point = PhysicalPoint { x: 200, y: 200 };
        let drag_host = window(1);
        let topmost_floating = window(2);
        let lower_floating = window(3);
        let main_window = window(4);
        let windows = [
            (drag_host, bounds()),
            (topmost_floating, bounds()),
            (lower_floating, bounds()),
            (main_window, bounds()),
        ];

        assert_eq!(
            resolve(
                point,
                &windows,
                &[topmost_floating, lower_floating, main_window],
                Some(drag_host),
            ),
            NativeWindowAtPoint::Known(topmost_floating),
        );
    }

    #[test]
    fn topmost_registered_dock_window_wins_when_floating_windows_overlap() {
        let point = PhysicalPoint { x: 200, y: 200 };
        let topmost_floating = window(1);
        let lower_main = window(2);
        let windows = [
            (topmost_floating, bounds()),
            (lower_main, bounds()),
        ];

        assert_eq!(
            resolve(point, &windows, &[topmost_floating, lower_main], None),
            NativeWindowAtPoint::Known(topmost_floating),
        );
    }

    #[test]
    fn windows_not_covering_the_pointer_do_not_occlude_a_target() {
        let point = PhysicalPoint { x: 200, y: 200 };
        let unrelated_window = window(1);
        let main_window = window(2);
        let outside = PhysicalRect {
            left: 600,
            top: 100,
            right: 900,
            bottom: 400,
        };
        let windows = [
            (unrelated_window, outside),
            (main_window, bounds()),
        ];

        assert_eq!(resolve(point, &windows, &[main_window], None), NativeWindowAtPoint::Known(main_window));
    }
}
