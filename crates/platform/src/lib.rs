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

/// The exact native frame styles captured before a temporary DragHost is made borderless.
///
/// Fields stay private so callers can only remove and restore the complete frame state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeWindowFrameState {
    style: u32,
    extended_style: u32,
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
pub enum NativeHorizontalResizeEdge {
    Left,
    Right,
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
    /// Returns the visible DWM frame bounds, excluding invisible resize borders.
    /// Platforms without a distinct visible frame may use the outer window rect.
    fn visible_frame_rect(&self, window: NativeWindowId) -> Result<PhysicalRect, NativeError> {
        self.window_rect(window)
    }
    fn client_origin(&self, window: NativeWindowId) -> Result<PhysicalPoint, NativeError>;
    /// Returns client-content dimensions in physical pixels, excluding the non-client frame.
    fn client_size(&self, window: NativeWindowId) -> Result<PhysicalSize, NativeError>;
    fn scale_factor(&self, window: NativeWindowId) -> Result<f32, NativeError>;
    fn move_window(
        &self,
        window: NativeWindowId,
        position: PhysicalPoint,
    ) -> Result<(), NativeError>;
    /// Moves a window to the top of the regular Z-order without activating it.
    ///
    /// The default preserves compatibility with platform adapters that do not
    /// expose a separate non-activating Z-order operation.
    fn move_window_to_top(
        &self,
        window: NativeWindowId,
        position: PhysicalPoint,
    ) -> Result<(), NativeError> {
        self.move_window(window, position)
    }
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

    /// Invalidates a native window so the platform message loop schedules a paint.
    /// The default keeps non-Windows adapters and test doubles platform-neutral.
    fn request_redraw(&self, _window: NativeWindowId) -> Result<(), NativeError> {
        Ok(())
    }

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

    /// Removes native caption and frame styles while preserving the current client rectangle.
    fn remove_window_frame(
        &self,
        window: NativeWindowId,
    ) -> Result<NativeWindowFrameState, NativeError>;
    /// Restores a previously captured native frame while preserving the current client rectangle.
    fn restore_window_frame(
        &self,
        window: NativeWindowId,
        state: NativeWindowFrameState,
    ) -> Result<(), NativeError>;

    fn enable_resize(&self, window: NativeWindowId) -> Result<(), NativeError>;
    fn disable_resize(&self, window: NativeWindowId) -> Result<(), NativeError>;
    /// Controls whether the native left edge and its corners initiate OS resizing.
    /// The right edge remains available independently.
    fn set_left_resize_enabled(
        &self,
        _window: NativeWindowId,
        _enabled: bool,
    ) -> Result<(), NativeError> {
        Ok(())
    }
    /// Controls whether the native right edge and its corners initiate OS resizing.
    fn set_right_resize_enabled(
        &self,
        _window: NativeWindowId,
        _enabled: bool,
    ) -> Result<(), NativeError> {
        Ok(())
    }
    /// Controls whether the top edge and top corners initiate OS resizing.
    fn set_top_resize_enabled(
        &self,
        _window: NativeWindowId,
        _enabled: bool,
    ) -> Result<(), NativeError> {
        Ok(())
    }
    /// Controls whether the bottom edge and bottom corners initiate OS resizing.
    fn set_bottom_resize_enabled(
        &self,
        _window: NativeWindowId,
        _enabled: bool,
    ) -> Result<(), NativeError> {
        Ok(())
    }
    /// Stores a WindowGrid's native Z priority in front-to-back order for
    /// activation-time correction. Implementations must preserve the same order
    /// when any member is activated, including vertical header-overlap constraints.
    fn set_window_chain_z_order(
        &self,
        _window: NativeWindowId,
        _ordered_members: &[NativeWindowId],
    ) -> Result<(), NativeError> {
        Ok(())
    }
    /// Sets the windows that follow this HWND when its right resize edge moves.
    ///
    /// The Windows adapter captures their origins at WM_ENTERSIZEMOVE and
    /// repositions them directly from each WM_SIZING proposal.
    fn set_window_resize_followers(
        &self,
        _window: NativeWindowId,
        _followers: &[NativeWindowId],
    ) -> Result<(), NativeError> {
        Ok(())
    }
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

    /// Sets client minimum widths for left- and right-edge native resizing.
    /// Values are physical pixels and exclude the non-client frame.
    fn set_horizontal_resize_min_widths(
        &self,
        _window: NativeWindowId,
        _minimum_client_widths: Option<(i32, i32)>,
    ) -> Result<(), NativeError> {
        Ok(())
    }

    /// Returns the horizontal side reported by the active native WM_SIZING message.
    /// None means the current sizing operation is vertical or no side is known.
    fn horizontal_resize_edge(
        &self,
        _window: NativeWindowId,
    ) -> Result<Option<NativeHorizontalResizeEdge>, NativeError> {
        Ok(None)
    }

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

#[cfg(any(windows, test))]
fn is_desktop_shell_class_name(class_name: &str) -> bool {
    matches!(class_name, "Progman" | "WorkerW")
}

#[cfg(windows)]
mod resize_loop_state;

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use super::resize_loop_state::ResizeLoopState;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::mem::size_of;
    use windows::core::BOOL;
    use windows::Win32::{
        Foundation::{
            GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError, WIN32_ERROR, WPARAM,
        },
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
            Gdi::{
                ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, InvalidateRect,
                MONITORINFO,
            },
        },
        Media::{timeBeginPeriod, timeEndPeriod},
        UI::{
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass},
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON},
            WindowsAndMessaging::*,
        },
    };

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

    /// Removes the native system-menu style so Windows does not draw its
    /// caption close button over the client area during a resize.
    ///
    /// Keep `WS_THICKFRAME`: GPUI Floating windows still need the native resize
    /// border even though their title bar is hidden.
    pub(super) fn hide_system_close_button(window: NativeWindowId) -> Result<(), NativeError> {
        let native = hwnd(window);
        let (_, client_origin, client_size) = client_geometry_for_frame_change(native)?;
        // SAFETY: The HWND belongs to the GPUI Floating window passed by the caller.
        let style = unsafe { GetWindowLongPtrW(native, GWL_STYLE) as u32 };
        if style & WS_SYSMENU.0 == 0 {
            return Ok(());
        }
        let extended_style = unsafe { GetWindowLongPtrW(native, GWL_EXSTYLE) as u32 };
        let updated_style = style & !WS_SYSMENU.0;

        if let Err(error) = write_window_frame_styles(native, updated_style, extended_style) {
            let _ = write_window_frame_styles(native, style, extended_style);
            return Err(error);
        }
        if let Err(error) = notify_native_frame_change(native) {
            let _ = write_window_frame_styles(native, style, extended_style);
            let _ = notify_native_frame_change(native);
            return Err(error);
        }
        if let Err(error) =
            preserve_client_geometry_after_frame_change(native, client_origin, client_size)
        {
            let _ = write_window_frame_styles(native, style, extended_style);
            let _ = notify_native_frame_change(native);
            return Err(error);
        }

        Ok(())
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

    fn client_geometry_for_frame_change(
        native: HWND,
    ) -> Result<(PhysicalRect, PhysicalPoint, PhysicalSize), NativeError> {
        let mut window_rect = RECT::default();
        let mut client_rect = RECT::default();
        // SAFETY: Both RECT values are writable outputs for this live GPUI HWND.
        unsafe {
            GetWindowRect(native, &mut window_rect)
                .map_err(|error| platform_error("GetWindowRect(frame)", error))?;
            GetClientRect(native, &mut client_rect)
                .map_err(|error| platform_error("GetClientRect(frame)", error))?;
        }

        let mut client_origin = POINT::default();
        // SAFETY: client_origin is a valid writable POINT in client coordinates.
        unsafe { ClientToScreen(native, &mut client_origin) }
            .ok()
            .map_err(|error| platform_error("ClientToScreen(frame)", error))?;

        let client_size = PhysicalSize {
            width: client_rect.right.checked_sub(client_rect.left)
                .ok_or(NativeError::InvalidSize)?,
            height: client_rect.bottom.checked_sub(client_rect.top)
                .ok_or(NativeError::InvalidSize)?,
        };
        if !client_size.is_valid() {
            return Err(NativeError::InvalidSize);
        }

        Ok((
            rect_from_native(window_rect),
            PhysicalPoint { x: client_origin.x, y: client_origin.y },
            client_size,
        ))
    }

    /// Corrects the outer HWND rectangle after a frame-style change so the client
    /// origin and dimensions remain exactly where they were before that change.
    fn preserve_client_geometry_after_frame_change(
        native: HWND,
        expected_origin: PhysicalPoint,
        expected_size: PhysicalSize,
    ) -> Result<(), NativeError> {
        let (window_rect, current_origin, current_size) =
            client_geometry_for_frame_change(native)?;
        let position = PhysicalPoint {
            x: window_rect.left.checked_add(
                expected_origin.x.checked_sub(current_origin.x)
                    .ok_or(NativeError::InvalidSize)?,
            ).ok_or(NativeError::InvalidSize)?,
            y: window_rect.top.checked_add(
                expected_origin.y.checked_sub(current_origin.y)
                    .ok_or(NativeError::InvalidSize)?,
            ).ok_or(NativeError::InvalidSize)?,
        };
        let outer_width = window_rect.width().checked_add(
            expected_size.width.checked_sub(current_size.width)
                .ok_or(NativeError::InvalidSize)?,
        ).ok_or(NativeError::InvalidSize)?;
        let outer_height = window_rect.height().checked_add(
            expected_size.height.checked_sub(current_size.height)
                .ok_or(NativeError::InvalidSize)?,
        ).ok_or(NativeError::InvalidSize)?;
        if outer_width <= 0 || outer_height <= 0 {
            return Err(NativeError::InvalidSize);
        }

        // SAFETY: Keep Z-order and activation unchanged while restoring the saved client rectangle.
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
        .map_err(|error| platform_error("SetWindowPos(preserve client geometry)", error))
    }

    fn write_window_frame_styles(
        native: HWND,
        style: u32,
        extended_style: u32,
    ) -> Result<(), NativeError> {
        // SAFETY: The HWND is owned by GPUI; GWL_STYLE accepts the supplied style bitmask.
        unsafe { SetLastError(WIN32_ERROR(0)); }
        let previous_style = unsafe { SetWindowLongPtrW(native, GWL_STYLE, style as isize) };
        if previous_style == 0 {
            let code = unsafe { GetLastError().0 };
            if code != 0 {
                return Err(NativeError::Platform {
                    operation: "SetWindowLongPtrW(style)",
                    code,
                });
            }
        }

        // SAFETY: The HWND is owned by GPUI; GWL_EXSTYLE accepts the supplied extended-style bitmask.
        unsafe { SetLastError(WIN32_ERROR(0)); }
        let previous_extended_style =
            unsafe { SetWindowLongPtrW(native, GWL_EXSTYLE, extended_style as isize) };
        if previous_extended_style == 0 {
            let code = unsafe { GetLastError().0 };
            if code != 0 {
                return Err(NativeError::Platform {
                    operation: "SetWindowLongPtrW(exstyle)",
                    code,
                });
            }
        }
        Ok(())
    }

    fn notify_native_frame_change(native: HWND) -> Result<(), NativeError> {
        // SAFETY: Recalculate only non-client metrics; preserve position, size, Z-order and activation.
        unsafe {
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
        .map_err(|error| platform_error("SetWindowPos(frame changed)", error))
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

    /// The input order is front-to-back. Raising in reverse leaves the first
    /// member on top while preserving every direction-aware grid overlap.
    fn raise_native_window_chain(ordered_members: &[HWND]) -> Result<(), NativeError> {
        for member in ordered_members.iter().rev().copied() {
            // SAFETY: All handles belong to live GPUI windows registered in this chain.
            unsafe {
                SetWindowPos(
                    member,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                )
            }
            .map_err(|error| platform_error("SetWindowPos(chain Z-order)", error))?;
        }
        Ok(())
    }

    fn hit_test_code_for_resize(
        hit_test: u32,
        left_resize_enabled: bool,
        right_resize_enabled: bool,
        top_resize_enabled: bool,
        bottom_resize_enabled: bool,
    ) -> i32 {
        if !left_resize_enabled && matches!(hit_test, HTLEFT | HTTOPLEFT | HTBOTTOMLEFT) {
            // A disabled shared left edge passes through to the previous chain member.
            HTTRANSPARENT
        } else if !right_resize_enabled
            && matches!(hit_test, HTRIGHT | HTTOPRIGHT | HTBOTTOMRIGHT)
        {
            // A disabled shared right edge passes through to the next chain member.
            HTTRANSPARENT
        } else if !top_resize_enabled && matches!(hit_test, HTTOP | HTTOPLEFT | HTTOPRIGHT) {
            // Keep the top border available to client hit-testing so the native
            // frame cannot start resizing through a joined composition header.
            HTCLIENT as i32
        } else if !bottom_resize_enabled
            && matches!(hit_test, HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT)
        {
            // A disabled shared bottom edge passes through to the lower grid member.
            HTTRANSPARENT
        } else {
            hit_test as i32
        }
    }

    fn horizontal_resize_edge_from_sizing_code(
        sizing_edge: u32,
    ) -> Option<NativeHorizontalResizeEdge> {
        match sizing_edge {
            1 | 4 | 7 => Some(NativeHorizontalResizeEdge::Left),
            2 | 5 | 8 => Some(NativeHorizontalResizeEdge::Right),
            _ => None,
        }
    }

    /// Adjusts a proposed outer RECT while preserving the opposite window edge.
    /// WMSZ_LEFT/TOPLEFT/BOTTOMLEFT use the left-composition minimum; right-side
    /// resize messages use the right-composition minimum.
    fn clamp_sizing_rect(
        rect: &mut RECT,
        sizing_edge: u32,
        minimum_outer_width: i32,
    ) -> bool {
        let anchor_left = match horizontal_resize_edge_from_sizing_code(sizing_edge) {
            Some(NativeHorizontalResizeEdge::Right) => true,
            Some(NativeHorizontalResizeEdge::Left) => false,
            None => return false,
        };
        if minimum_outer_width <= 0 {
            return false;
        }

        let proposed_width = rect.right.saturating_sub(rect.left);
        if proposed_width >= minimum_outer_width {
            return false;
        }

        if anchor_left {
            rect.right = rect.left.saturating_add(minimum_outer_width);
        } else {
            rect.left = rect.right.saturating_sub(minimum_outer_width);
        }
        true
    }

    unsafe fn clamp_native_sizing_rect(
        window: HWND,
        sizing_edge: u32,
        lparam: LPARAM,
        state: &ResizeLoopState,
    ) -> bool {
        if lparam.0 == 0 {
            return false;
        }

        let minimum_client_width = match horizontal_resize_edge_from_sizing_code(sizing_edge) {
            Some(NativeHorizontalResizeEdge::Left) => state.left_min_client_width,
            Some(NativeHorizontalResizeEdge::Right) => state.right_min_client_width,
            None => return false,
        };
        if minimum_client_width <= 0 {
            return false;
        }

        let mut window_rect = RECT::default();
        let mut client_rect = RECT::default();
        // SAFETY: Both RECT outputs belong to this live HWND.
        if unsafe { GetWindowRect(window, &mut window_rect) }.is_err()
            || unsafe { GetClientRect(window, &mut client_rect) }.is_err()
        {
            return false;
        }
        let outer_width = window_rect.right.saturating_sub(window_rect.left);
        let client_width = client_rect.right.saturating_sub(client_rect.left);
        let frame_width = outer_width.saturating_sub(client_width).max(0);
        let minimum_outer_width = minimum_client_width.saturating_add(frame_width);

        // SAFETY: WM_SIZING supplies a writable RECT pointer through lParam.
        let proposed = unsafe { &mut *(lparam.0 as *mut RECT) };
        clamp_sizing_rect(proposed, sizing_edge, minimum_outer_width)
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
            let mut initial_window_rect = RECT::default();
            let resize_start_right =
                if unsafe { GetWindowRect(window, &mut initial_window_rect) }.is_ok() {
                    Some(initial_window_rect.right)
                } else {
                    None
                };
            let configured_followers = unsafe { (*state).resize_followers.clone() };
            let mut resize_follower_origins =
                Vec::with_capacity(configured_followers.len());
            for follower in configured_followers {
                let mut follower_rect = RECT::default();
                if unsafe { GetWindowRect(follower, &mut follower_rect) }.is_ok() {
                    resize_follower_origins.push((
                        follower,
                        POINT {
                            x: follower_rect.left,
                            y: follower_rect.top,
                        },
                    ));
                }
            }

            // SAFETY: The subclass owns this live state allocation.
            unsafe {
                (*state).active = true;
                (*state).horizontal_edge = None;
                (*state).resize_start_right = resize_start_right;
                (*state).resize_follower_origins = resize_follower_origins;
            };
        }
        if message == WM_SIZING {
            let edge = horizontal_resize_edge_from_sizing_code(wparam.0 as u32);
            // SAFETY: The subclass owns this live state allocation.
            unsafe { (*state).horizontal_edge = edge };
        }

        // SAFETY: Forward the message to GPUI/the remaining subclass chain.
        let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };

        if message == WM_ACTIVATE && (wparam.0 & 0xffff) != WA_INACTIVE as usize {
            // Windows normally raises the activated HWND above its siblings.
            // Reapply the grid's direction-aware stacking without changing focus,
            // so upper headers and shared horizontal resize edges remain covered.
            // SAFETY: The subclass owns this state for the lifetime of the HWND.
            let chain_z_order = unsafe { (*state).chain_z_order.clone() };
            if !chain_z_order.is_empty() {
                let _ = raise_native_window_chain(&chain_z_order);
            }
        }

        if message == WM_NCHITTEST {
            // A secondary Window Chain member must not expose a left resize
            // handle: that edge is the join with its preceding native window.
            // SAFETY: The subclass owns this state for the lifetime of the HWND.
            let (
                left_resize_enabled,
                right_resize_enabled,
                top_resize_enabled,
                bottom_resize_enabled,
            ) = unsafe {
                (
                    (*state).left_resize_enabled,
                    (*state).right_resize_enabled,
                    (*state).top_resize_enabled,
                    (*state).bottom_resize_enabled,
                )
            };
            let hit_test = result.0 as u32;
            let adjusted_hit_test = hit_test_code_for_resize(
                hit_test,
                left_resize_enabled,
                right_resize_enabled,
                top_resize_enabled,
                bottom_resize_enabled,
            );
            if adjusted_hit_test != hit_test as i32 {
                return LRESULT(adjusted_hit_test as isize);
            }
        }

        // WM_SIZING carries a mutable outer-window RECT. Clamp it after the
        // downstream handler so no later handler can undo the composition floor.
        let sizing_was_clamped = message == WM_SIZING
            // SAFETY: This subclass owns the live ResizeLoopState until WM_NCDESTROY.
            && unsafe {
                clamp_native_sizing_rect(window, wparam.0 as u32, lparam, &*state)
            };

        if message == WM_SIZING
            && horizontal_resize_edge_from_sizing_code(wparam.0 as u32)
                == Some(NativeHorizontalResizeEdge::Right)
            && lparam.0 != 0
        {
            // WM_SIZING supplies the final proposed outer RECT after downstream
            // handlers and our minimum-width clamp. Move followers from their
            // WM_ENTERSIZEMOVE origins, never from their previous sampled positions.
            let proposed = unsafe { &*(lparam.0 as *const RECT) };
            let (resize_start_right, follower_origins) = unsafe {
                let state = &*state;
                (
                    state.resize_start_right,
                    state.resize_follower_origins.clone(),
                )
            };
            if let Some(resize_start_right) = resize_start_right {
                let delta_x = proposed.right.saturating_sub(resize_start_right);
                for (follower, origin) in follower_origins {
                    let _ = unsafe {
                        SetWindowPos(
                            follower,
                            None,
                            origin.x.saturating_add(delta_x),
                            origin.y,
                            0,
                            0,
                            SWP_NOSIZE
                                | SWP_NOZORDER
                                | SWP_NOACTIVATE
                                | SWP_NOOWNERZORDER,
                        )
                    };
                    
                }
            }
        }

        if message == WM_EXITSIZEMOVE {
            // SAFETY: The subclass owns this live state allocation.
            unsafe {
                (*state).active = false;
                (*state).horizontal_edge = None;
                (*state).resize_start_right = None;
                (*state).resize_follower_origins.clear();
            };
        }
        if message == WM_NCDESTROY {
            // SAFETY: Detach this callback before freeing its owned state.
            unsafe {
                let _ = RemoveWindowSubclass(window, Some(resize_loop_subclass_proc), subclass_id);
                drop(Box::from_raw(state));
            }
        }

        if sizing_was_clamped {
            LRESULT(1)
        } else {
            result
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

        fn visible_frame_rect(&self, window: NativeWindowId) -> Result<PhysicalRect, NativeError> {
            let mut rect = RECT::default();
            // DWM reports the visible frame without the invisible resize border
            // included by GetWindowRect. This is the edge users see when windows
            // are placed next to one another.
            // SAFETY: rect is writable and window is a live GPUI-owned HWND.
            if unsafe {
                DwmGetWindowAttribute(
                    hwnd(window),
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    &mut rect as *mut RECT as *mut _,
                    size_of::<RECT>() as u32,
                )
            }
            .is_ok()
            {
                return Ok(rect_from_native(rect));
            }

            self.window_rect(window)
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

        fn move_window_to_top(
            &self,
            window: NativeWindowId,
            position: PhysicalPoint,
        ) -> Result<(), NativeError> {
            // Keep the temporary drag surface above sibling Floating windows while
            // leaving activation and the owner's Z-order unchanged. Drop routing
            // skips this HWND and continues hit-testing the window underneath it.
            // SAFETY: The HWND is a live GPUI-owned window and position is valid.
            unsafe {
                SetWindowPos(
                    hwnd(window),
                    Some(HWND_TOP),
                    position.x,
                    position.y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                )
            }
            .map_err(|error| platform_error("SetWindowPos(move to top)", error))
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

        fn request_redraw(&self, window: NativeWindowId) -> Result<(), NativeError> {
            // Queue a normal WM_PAINT for the live GPUI window. Do not use
            // GPUI's private force-update message: it requires an internal
            // per-process validation token that this platform adapter cannot access.
            // SAFETY: window is a GPUI-owned HWND.
            let invalidated = unsafe { InvalidateRect(Some(hwnd(window)), None, false) };
            if !invalidated.as_bool() {
                return Err(last_error("InvalidateRect"));
            }
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

        fn remove_window_frame(
            &self,
            window: NativeWindowId,
        ) -> Result<NativeWindowFrameState, NativeError> {
            let native = hwnd(window);
            let (_, client_origin, client_size) = client_geometry_for_frame_change(native)?;
            // SAFETY: Read the current styles of this live GPUI-owned HWND.
            let style = unsafe { GetWindowLongPtrW(native, GWL_STYLE) as u32 };
            let extended_style = unsafe { GetWindowLongPtrW(native, GWL_EXSTYLE) as u32 };
            let original = NativeWindowFrameState { style, extended_style };

            let borderless_style = style
                & !(WS_CAPTION.0 | WS_BORDER.0 | WS_DLGFRAME.0 | WS_THICKFRAME.0);
            let borderless_extended_style = extended_style
                & !(WS_EX_WINDOWEDGE.0
                    | WS_EX_CLIENTEDGE.0
                    | WS_EX_STATICEDGE.0
                    | WS_EX_DLGMODALFRAME.0);
            if let Err(error) = write_window_frame_styles(
                native,
                borderless_style,
                borderless_extended_style,
            ) {
                let _ = write_window_frame_styles(native, original.style, original.extended_style);
                return Err(error);
            }
            if let Err(error) = notify_native_frame_change(native) {
                let _ = write_window_frame_styles(native, original.style, original.extended_style);
                let _ = notify_native_frame_change(native);
                return Err(error);
            }
            if let Err(error) = preserve_client_geometry_after_frame_change(
                native,
                client_origin,
                client_size,
            ) {
                let _ = write_window_frame_styles(native, original.style, original.extended_style);
                let _ = notify_native_frame_change(native);
                return Err(error);
            }

            Ok(original)
        }

        fn restore_window_frame(
            &self,
            window: NativeWindowId,
            state: NativeWindowFrameState,
        ) -> Result<(), NativeError> {
            let native = hwnd(window);
            let (_, client_origin, client_size) = client_geometry_for_frame_change(native)?;
            write_window_frame_styles(native, state.style, state.extended_style)?;
            notify_native_frame_change(native)?;
            preserve_client_geometry_after_frame_change(native, client_origin, client_size)
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
            let tracking_installed = unsafe {
                GetWindowSubclass(
                    native,
                    Some(resize_loop_subclass_proc),
                    RESIZE_LOOP_SUBCLASS_ID,
                    Some(&mut reference_data as *mut usize),
                )
            }
            .as_bool();
            if !tracking_installed {
                let pointer = Box::into_raw(Box::new(ResizeLoopState {
                    active: false,
                    horizontal_edge: None,
                    left_resize_enabled: true,
                    right_resize_enabled: true,
                    top_resize_enabled: true,
                    bottom_resize_enabled: true,
                    chain_z_order: Vec::new(),
                    resize_followers: Vec::new(),
                    resize_follower_origins: Vec::new(),
                    resize_start_right: None,
                    left_min_client_width: 0,
                    right_min_client_width: 0,
                }));
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
            }

            Ok(())
        }

        fn disable_resize(&self, window: NativeWindowId) -> Result<(), NativeError> {
            let native = hwnd(window);
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

        fn set_left_resize_enabled(
            &self,
            window: NativeWindowId,
            enabled: bool,
        ) -> Result<(), NativeError> {
            let mut reference_data = 0usize;
            // SAFETY: Query the resize-loop subclass installed by enable_resize.
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
                return Err(NativeError::Unsupported);
            }

            // SAFETY: This state remains owned by the live HWND subclass.
            unsafe {
                (*(reference_data as *mut ResizeLoopState)).left_resize_enabled = enabled;
            }
            Ok(())
        }

        fn set_right_resize_enabled(
            &self,
            window: NativeWindowId,
            enabled: bool,
        ) -> Result<(), NativeError> {
            let mut reference_data = 0usize;
            // SAFETY: Query the resize-loop subclass installed by enable_resize.
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
                return Err(NativeError::Unsupported);
            }

            // SAFETY: This state remains owned by the live HWND subclass.
            unsafe {
                (*(reference_data as *mut ResizeLoopState)).right_resize_enabled = enabled;
            }
            Ok(())
        }

        fn set_top_resize_enabled(
            &self,
            window: NativeWindowId,
            enabled: bool,
        ) -> Result<(), NativeError> {
            let mut reference_data = 0usize;
            // SAFETY: Query the resize-loop subclass installed by enable_resize.
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
                return Err(NativeError::Unsupported);
            }

            // SAFETY: This state remains owned by the live HWND subclass.
            unsafe {
                (*(reference_data as *mut ResizeLoopState)).top_resize_enabled = enabled;
            }
            Ok(())
        }

        fn set_bottom_resize_enabled(
            &self,
            window: NativeWindowId,
            enabled: bool,
        ) -> Result<(), NativeError> {
            let mut reference_data = 0usize;
            // SAFETY: Query the resize-loop subclass installed by enable_resize.
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
                return Err(NativeError::Unsupported);
            }

            // SAFETY: This state remains owned by the live HWND subclass.
            unsafe {
                (*(reference_data as *mut ResizeLoopState)).bottom_resize_enabled = enabled;
            }
            Ok(())
        }

        fn set_window_chain_z_order(
            &self,
            window: NativeWindowId,
            ordered_members: &[NativeWindowId],
        ) -> Result<(), NativeError> {
            let mut reference_data = 0usize;
            // SAFETY: Query the resize-loop subclass installed by enable_resize.
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
                return Err(NativeError::Unsupported);
            }

            let native_members = ordered_members.iter().copied().map(hwnd).collect();
            // SAFETY: The state belongs to this live HWND subclass.
            unsafe {
                (*(reference_data as *mut ResizeLoopState)).chain_z_order = native_members;
            }
            Ok(())
        }

        fn set_window_resize_followers(
            &self,
            window: NativeWindowId,
            followers: &[NativeWindowId],
        ) -> Result<(), NativeError> {
            let mut reference_data = 0usize;
            // SAFETY: Query the resize-loop subclass installed on this HWND.
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
                return Err(NativeError::Unsupported);
            }

            let native_followers = followers
                .iter()
                .copied()
                .map(hwnd)
                .filter(|follower| *follower != hwnd(window))
                .collect();
            // SAFETY: The state belongs to this live HWND subclass.
            unsafe {
                (*(reference_data as *mut ResizeLoopState)).resize_followers = native_followers;
            }
            Ok(())
        }

        fn set_horizontal_resize_min_widths(
            &self,
            window: NativeWindowId,
            minimum_client_widths: Option<(i32, i32)>,
        ) -> Result<(), NativeError> {
            if minimum_client_widths.is_some_and(|(left, right)| left <= 0 || right <= 0) {
                return Err(NativeError::InvalidSize);
            }

            let native = hwnd(window);
            let mut reference_data = 0usize;
            // SAFETY: Query the resize-loop subclass installed by enable_resize.
            let installed = unsafe {
                GetWindowSubclass(
                    native,
                    Some(resize_loop_subclass_proc),
                    RESIZE_LOOP_SUBCLASS_ID,
                    Some(&mut reference_data as *mut usize),
                )
            }
            .as_bool();
            if !installed || reference_data == 0 {
                return Err(NativeError::Unsupported);
            }

            let (left_width, right_width) = minimum_client_widths.unwrap_or((0, 0));
            // SAFETY: This allocation is owned by the live HWND subclass.
            unsafe {
                (*(reference_data as *mut ResizeLoopState)).left_min_client_width = left_width;
                (*(reference_data as *mut ResizeLoopState)).right_min_client_width = right_width;
            }
            Ok(())
        }

        fn horizontal_resize_edge(
            &self,
            window: NativeWindowId,
        ) -> Result<Option<NativeHorizontalResizeEdge>, NativeError> {
            let mut reference_data = 0usize;
            // SAFETY: Query the resize-loop subclass installed by enable_resize.
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
                return Ok(None);
            }

            // SAFETY: This state remains owned by the live HWND subclass.
            Ok(unsafe {
                (*(reference_data as *const ResizeLoopState)).horizontal_edge
            })
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

                        let mut class_name = [0u16; 64];
                        // SAFETY: class_name is a valid writable UTF-16 buffer.
                        let class_name_len = unsafe {
                            GetClassNameW(native, &mut class_name)
                        };
                        if class_name_len > 0
                            && is_desktop_shell_class_name(
                                &String::from_utf16_lossy(
                                    &class_name[..class_name_len as usize],
                                ),
                            )
                        {
                            // Progman/WorkerW are the desktop surface, not an
                            // application window that should block a floating tear-off.
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

    #[cfg(test)]
    mod native_resize_sizing_tests {
        use super::*;

        #[test]
        fn native_sizing_codes_report_the_selected_horizontal_edge() {
            assert_eq!(
                horizontal_resize_edge_from_sizing_code(1),
                Some(NativeHorizontalResizeEdge::Left)
            );
            assert_eq!(
                horizontal_resize_edge_from_sizing_code(4),
                Some(NativeHorizontalResizeEdge::Left)
            );
            assert_eq!(
                horizontal_resize_edge_from_sizing_code(7),
                Some(NativeHorizontalResizeEdge::Left)
            );
            assert_eq!(
                horizontal_resize_edge_from_sizing_code(2),
                Some(NativeHorizontalResizeEdge::Right)
            );
            assert_eq!(
                horizontal_resize_edge_from_sizing_code(5),
                Some(NativeHorizontalResizeEdge::Right)
            );
            assert_eq!(
                horizontal_resize_edge_from_sizing_code(8),
                Some(NativeHorizontalResizeEdge::Right)
            );
            assert_eq!(horizontal_resize_edge_from_sizing_code(3), None);
            assert_eq!(horizontal_resize_edge_from_sizing_code(6), None);
        }

        #[test]
        fn disabling_left_resize_passes_overlapping_edges_to_the_previous_window() {
            for hit_test in [HTLEFT, HTTOPLEFT, HTBOTTOMLEFT] {
                assert_eq!(
                    hit_test_code_for_resize(hit_test, false, true, true, true),
                    HTTRANSPARENT
                );
            }
            assert_eq!(
                hit_test_code_for_resize(HTLEFT, true, true, true, true),
                HTLEFT as i32
            );
            assert_eq!(
                hit_test_code_for_resize(HTRIGHT, false, true, true, true),
                HTRIGHT as i32
            );
            assert_eq!(
                hit_test_code_for_resize(HTBOTTOMRIGHT, true, true, true, true),
                HTBOTTOMRIGHT as i32
            );
        }

        #[test]
        fn disabling_top_resize_keeps_the_header_corners_in_client_area() {
            for hit_test in [HTTOP, HTTOPLEFT, HTTOPRIGHT] {
                assert_eq!(
                    hit_test_code_for_resize(hit_test, true, true, false, true),
                    HTCLIENT as i32
                );
            }

            assert_eq!(
                hit_test_code_for_resize(HTTOP, true, true, true, true),
                HTTOP as i32
            );
            assert_eq!(
                hit_test_code_for_resize(HTTOPRIGHT, true, true, false, true),
                HTCLIENT as i32
            );
            assert_eq!(
                hit_test_code_for_resize(HTBOTTOMRIGHT, true, true, true, true),
                HTBOTTOMRIGHT as i32
            );
        }

        #[test]
        fn disabling_bottom_resize_passes_the_shared_edge_to_the_lower_window() {
            for hit_test in [HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT] {
                assert_eq!(
                    hit_test_code_for_resize(hit_test, true, true, true, false),
                    HTTRANSPARENT
                );
            }

            assert_eq!(
                hit_test_code_for_resize(HTBOTTOM, true, true, true, true),
                HTBOTTOM as i32
            );
        }

        #[test]
        fn secondary_top_left_corner_still_passes_through_to_the_previous_window() {
            assert_eq!(
                hit_test_code_for_resize(HTTOPLEFT, false, true, false, true),
                HTTRANSPARENT
            );
        }

        #[test]
        fn left_edge_minimum_preserves_the_right_side_of_the_window() {
            let mut rect = RECT {
                left: 100,
                top: 20,
                right: 500,
                bottom: 400,
            };
            assert!(clamp_sizing_rect(&mut rect, 1, 560));
            assert_eq!(rect.left, -60);
            assert_eq!(rect.right, 500);
            assert_eq!(rect.right - rect.left, 560);
        }

        #[test]
        fn right_edge_minimum_preserves_the_left_side_of_the_window() {
            let mut rect = RECT {
                left: 100,
                top: 20,
                right: 500,
                bottom: 400,
            };
            assert!(clamp_sizing_rect(&mut rect, 2, 560));
            assert_eq!(rect.left, 100);
            assert_eq!(rect.right, 660);
            assert_eq!(rect.right - rect.left, 560);
        }

        #[test]
        fn vertical_resize_does_not_apply_a_horizontal_minimum() {
            let mut rect = RECT {
                left: 100,
                top: 20,
                right: 500,
                bottom: 400,
            };
            assert!(!clamp_sizing_rect(&mut rect, 6, 560));
            assert_eq!(rect.left, 100);
            assert_eq!(rect.right, 500);
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
        fn remove_window_frame(
            &self,
            _: NativeWindowId,
        ) -> Result<NativeWindowFrameState, NativeError> {
            Err(NativeError::Unsupported)
        }
        fn restore_window_frame(
            &self,
            _: NativeWindowId,
            _: NativeWindowFrameState,
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

/// Removes the system close button from a border-resizable Floating window.
#[cfg(windows)]
pub fn hide_system_close_button(window: NativeWindowId) -> Result<(), NativeError> {
    windows_impl::hide_system_close_button(window)
}

/// Native system close-button styling is only available on Windows.
#[cfg(not(windows))]
pub fn hide_system_close_button(_: NativeWindowId) -> Result<(), NativeError> {
    Err(NativeError::Unsupported)
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
    fn desktop_shell_classes_are_not_treated_as_foreign_application_windows() {
        assert!(super::is_desktop_shell_class_name("Progman"));
        assert!(super::is_desktop_shell_class_name("WorkerW"));
        assert!(!super::is_desktop_shell_class_name("Chrome_WidgetWin_1"));
        assert!(!super::is_desktop_shell_class_name(""));
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
