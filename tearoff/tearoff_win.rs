use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use windows::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT, WPARAM},
    Graphics::Gdi::ClientToScreen,
    Media::timeBeginPeriod,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON},
        WindowsAndMessaging::*,
    },
};

const RESIZE_REDRAW_TIMER_ID: usize = 2;
const RESIZE_REDRAW_INTERVAL_MS: u32 = 8;

// Из gpui_windows этого exact commit.
const WM_GPUI_FORCE_UPDATE_WINDOW: u32 = WM_USER + 5;

fn to_hwnd(value: isize) -> HWND {
    HWND(value as _)
}

/// Точность системного таймера 1 мс (нужна для ровного 4-мс опроса).
pub fn begin_high_res_timer() {
    unsafe {
        let _ = timeBeginPeriod(1);
    }
}

pub fn hwnd_of(window: &gpui::Window) -> Option<isize> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

// ------------------------------------------------------------
// INPUT
// ------------------------------------------------------------

pub fn cursor_pos() -> (i32, i32) {
    let mut point = POINT::default();

    unsafe {
        let _ = GetCursorPos(&mut point);
    }

    (point.x, point.y)
}

pub fn left_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16) & 0x8000 != 0 }
}

// ------------------------------------------------------------
// GEOMETRY
// ------------------------------------------------------------

/// (left, top, right, bottom) в экранных координатах.
pub fn window_rect(hwnd: isize) -> (i32, i32, i32, i32) {
    let mut rect = RECT::default();

    unsafe {
        let _ = GetWindowRect(to_hwnd(hwnd), &mut rect);
    }

    (rect.left, rect.top, rect.right, rect.bottom)
}

/// Левый верхний угол клиентской области в экранных координатах.
pub fn client_origin(hwnd: isize) -> (i32, i32) {
    let mut point = POINT::default();

    unsafe {
        let _ = ClientToScreen(to_hwnd(hwnd), &mut point);
    }

    (point.x, point.y)
}

/// Текущий масштаб окна (DPI / 96), как у `window.scale_factor()` в GPUI.
pub fn scale_of(hwnd: isize) -> f32 {
    let dpi = unsafe { GetDpiForWindow(to_hwnd(hwnd)) };

    if dpi == 0 {
        1.0
    } else {
        dpi as f32 / 96.0
    }
}

// ------------------------------------------------------------
// WINDOW OPS
// ------------------------------------------------------------

pub fn move_window(hwnd: isize, x: i32, y: i32) {
    unsafe {
        let _ = SetWindowPos(
            to_hwnd(hwnd),
            Some(HWND_TOP),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Меняет только размер окна (позицию и z-order не трогает).
pub fn resize_window(hwnd: isize, width: i32, height: i32) {
    unsafe {
        let _ = SetWindowPos(
            to_hwnd(hwnd),
            None,
            0,
            0,
            width,
            height,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub fn show_window(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(to_hwnd(hwnd), SW_SHOWNOACTIVATE);
    }
}

pub fn hide_window(hwnd: isize) {
    unsafe {
        let _ = ShowWindow(to_hwnd(hwnd), SW_HIDE);
    }
}

/// Делает `owner_hwnd` владельцем окна: оно всегда поверх владельца
/// и не появляется отдельной кнопкой на панели задач.
pub fn set_owner(window_hwnd: isize, owner_hwnd: isize) {
    unsafe {
        let _ = SetWindowLongPtrW(to_hwnd(window_hwnd), GWLP_HWNDPARENT, owner_hwnd);
    }
}

// ------------------------------------------------------------
// RESIZE
// ------------------------------------------------------------

/// Включает нативный resize/maximize и перерисовку во время изменения размера.
pub fn enable_resize(hwnd: isize) {
    let hwnd = to_hwnd(hwnd);

    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 | WS_THICKFRAME.0;
        let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);

        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );

        let _ = SetTimer(
            Some(hwnd),
            RESIZE_REDRAW_TIMER_ID,
            RESIZE_REDRAW_INTERVAL_MS,
            Some(resize_redraw_timer_proc),
        );
    }
}

/// Пока зажата ЛКМ, принудительно просим GPUI перерисовать окно
/// (иначе во время модального resize-цикла Windows картинка замирает).
unsafe extern "system" fn resize_redraw_timer_proc(hwnd: HWND, _msg: u32, _id: usize, _time: u32) {
    if left_down() {
        let _ = PostMessageW(
            Some(hwnd),
            WM_GPUI_FORCE_UPDATE_WINDOW,
            WPARAM(0),
            LPARAM(0),
        );
    }
}
