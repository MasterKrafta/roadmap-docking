use std::time::{Duration, Instant};

use gpui::{
    div, point, prelude::*, px, rgb, size, App, Bounds, Context, Entity, MouseButton, Window, WindowBounds,
    WindowKind, WindowOptions,
};


const DOCK_W: f32 = 260.0;
const PANEL_H: f32 = 180.0;

/// Период опроса курсора во время перетаскивания.
const TICK_MS: u64 = 4;

/// Сколько пикселей надо протащить, чтобы панель оторвалась от дока.
const DRAG_THRESHOLD: i32 = 2;

#[derive(Clone, Copy)]
struct Drag {
    start: (i32, i32),
    /// Смещение курсора относительно левого верхнего угла перетаскиваемого окна.
    offset: (i32, i32),
}

/// Единственный источник правды: и док, и панель читают состояние отсюда,
/// а `MainView` подписан на него через `cx.observe`.
#[derive(Default)]
struct DockState {
    drag: Option<Drag>,
    /// Курсор над доком: подсветка + «приклеить при отпускании».
    highlight: bool,

    main_hwnd: isize,

    /// HWND floating-окна. Создаётся один раз и переиспользуется.
    floating: Option<isize>,
    /// Floating-окно сейчас показано. Если false — панель в доке.
    floating_visible: bool,
}

impl DockState {
    fn begin_drag(&mut self, cx: &mut Context<Self>) {
        // Floating-окно ещё не создано — перетаскивать нечего.
        let Some(floating) = self.floating else {
            return;
        };

        let cursor = tearoff_win::cursor_pos();

        let (origin_x, origin_y) = if self.floating_visible {
            let (left, top, _, _) = tearoff_win::window_rect(floating);
            (left, top)
        } else {
            tearoff_win::client_origin(self.main_hwnd)
        };

        let already_dragging = self.drag.is_some();

        self.drag = Some(Drag {
            start: cursor,
            offset: (cursor.0 - origin_x, cursor.1 - origin_y),
        });

        // Опрос живёт только пока идёт drag.
        if !already_dragging {
            Self::spawn_polling(cx);
        }
    }

    fn spawn_polling(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(TICK_MS))
                .await;

            if !matches!(this.update(cx, |state, cx| state.tick(cx)), Ok(true)) {
                break;
            }
        })
        .detach();
    }

    /// Один шаг перетаскивания. `false` — опрос можно остановить.
    fn tick(&mut self, cx: &mut Context<Self>) -> bool {
        if !tearoff_win::left_down() {
            self.finish(cx);
            return false;
        }

        let (Some(drag), Some(hwnd)) = (self.drag, self.floating) else {
            return false;
        };

        let cursor = tearoff_win::cursor_pos();

        // DOCKED -> FLOATING: ждём, пока курсор отъедет дальше порога.
        if !self.floating_visible {
            let distance = (cursor.0 - drag.start.0)
                .abs()
                .max((cursor.1 - drag.start.1).abs());

            if distance < DRAG_THRESHOLD {
                return true;
            }
        }

        tearoff_win::move_window(hwnd, cursor.0 - drag.offset.0, cursor.1 - drag.offset.1);

        if !self.floating_visible {
            tearoff_win::show_window(hwnd);
            self.floating_visible = true;
            self.highlight = false;
            cx.notify();
        }

        // Попадание в док. DPI берём в момент проверки: окно могли
        // перетащить на монитор с другим масштабом.
        if self.main_hwnd == 0 {
            return true;
        }

        let (left, top, _, bottom) = tearoff_win::window_rect(self.main_hwnd);
        let dock_right = left + (DOCK_W * tearoff_win::scale_of(self.main_hwnd)) as i32;

        let over_dock =
            cursor.0 >= left && cursor.0 < dock_right && cursor.1 >= top && cursor.1 < bottom;

        if self.highlight != over_dock {
            self.highlight = over_dock;
            cx.notify();
        }

        true
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        if self.highlight {
            if let Some(hwnd) = self.floating {
                tearoff_win::hide_window(hwnd);
            }

            self.floating_visible = false;
        }

        self.drag = None;
        self.highlight = false;

        cx.notify();
    }
}

/// Верхняя полоска хедера floating-окна, занятая нативной resize-рамкой
/// (SM_CYSIZEFRAME + SM_CXPADDEDBORDER ≈ 8 px при 100% масштаба).
const RESIZE_ZONE: f32 = 8.0;

pub(crate) struct PanelView {
    pub(crate) dock: Entity<DockState>,
}

impl Render for PanelView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let dock = self.dock.clone();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x2b2b2b))
            .border_1()
            .border_color(rgb(0x555555))
            .child(
                div()
                    .id("panel-header")
                    .h(px(28.0))
                    .w_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .bg(rgb(0x3c3c3c))
                    .text_color(rgb(0xdddddd))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                        // Пока floating-окно на экране, докнутая копия панели не
                        // рисуется, значит клик пришёл из floating. Верхняя рамка
                        // у него — это resize, а не drag.
                        if dock.read(cx).floating_visible && event.position.y < px(RESIZE_ZONE) {
                            return;
                        }

                        dock.update(cx, |dock, cx| dock.begin_drag(cx));
                    })
                    .child("Brushes — тяни за заголовок"),
            )
            .child(
                div()
                    .p_3()
                    .text_color(rgb(0xaaaaaa))
                    .child("Содержимое панели"),
            )
    }
}

// ------------------------------------------------------------
// FLOATING WINDOW
// ------------------------------------------------------------

/// Создаёт floating-окно (скрытым). Вызывается ДО главного окна, поэтому
/// owner здесь не ставится: это делает `attach_floating`.
fn create_floating(cx: &mut App, state: &Entity<DockState>) {
    let options = WindowOptions {
        titlebar: None,
        // PopUp сразу даёт TOOLWINDOW-стиль: нет кнопки на панели задач.
        kind: WindowKind::PopUp,
        // Окно создаётся скрытым и остаётся скрытым, пока панель в доке.
        show: false,
        focus: false,
        is_resizable: true,
        is_minimizable: false,
        is_movable: false,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(DOCK_W), px(PANEL_H)),
        })),
        ..Default::default()
    };

    let handle = {
        let state = state.clone();

        cx.open_window(options, move |_, cx| cx.new(|_| PanelView { dock: state }))
    };

    let handle = match handle {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("Не удалось открыть floating window: {err}");
            return;
        }
    };

    let hwnd = handle.update(cx, |_, window, _| {
        let hwnd = tearoff_win::hwnd_of(window)?;

        // Нативный resize (WS_THICKFRAME) + maximize.
        tearoff_win::enable_resize(hwnd);

        Some(hwnd)
    });

    let Ok(Some(hwnd)) = hwnd else {
        eprintln!("Не удалось получить floating HWND");
        return;
    };

    // На случай, если платформа всё же показала окно при создании.
    tearoff_win::hide_window(hwnd);

    state.update(cx, |state, _| state.floating = Some(hwnd));
}

/// Вызывается ПОСЛЕ создания главного окна: назначает owner и размер по его DPI.
fn attach_floating(cx: &mut App, state: &Entity<DockState>) {
    let (main_hwnd, floating) = {
        let state = state.read(cx);
        (state.main_hwnd, state.floating)
    };

    let Some(floating) = floating else {
        return;
    };

    if main_hwnd == 0 {
        eprintln!("main HWND ещё не готов");
        return;
    }

    let scale = tearoff_win::scale_of(main_hwnd);

    tearoff_win::set_owner(floating, main_hwnd);
    tearoff_win::resize_window(floating, (DOCK_W * scale) as i32, (PANEL_H * scale) as i32);
}

// ------------------------------------------------------------
// MAIN VIEW
// ------------------------------------------------------------

struct MainView {
    state: Entity<DockState>,
    docked_panel: Entity<PanelView>,
    /// Для замера времени до первого render.
    started: Instant,
    first_render_logged: bool,
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_render_logged {
            self.first_render_logged = true;
            eprintln!(
                "[timing] первый render главного: {:?}",
                self.started.elapsed()
            );
        }

        let (docked, highlight) = {
            let state = self.state.read(cx);
            (!state.floating_visible, state.highlight)
        };

        let dock = div()
            .w(px(DOCK_W))
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(0x555555))
            .bg(rgb(if highlight { 0x1f4f8f } else { 0x202020 }))
            .when(docked, |dock| {
                dock.child(div().flex_1().child(self.docked_panel.clone()))
            });

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(rgb(0x1a1a1a))
            .child(dock)
            .child(
                div()
                    .flex_1()
                    .p_4()
                    .text_color(rgb(0x777777))
                    .child("Рабочая область"),
            )
    }
}

// ------------------------------------------------------------
// MAIN
// ------------------------------------------------------------

fn main() {
    tearoff_win::begin_high_res_timer();

    gpui_platform::application().run(|cx: &mut App| {
        let started = Instant::now();
        let state = cx.new(|_| DockState::default());

        // 1. Сначала floating: его стоимость платится до появления главного окна.
        create_floating(cx, &state);
        eprintln!("[timing] floating создан: {:?}", started.elapsed());

        // 2. Потом главное окно.
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(200.0), px(120.0)),
                size: size(px(1000.0), px(640.0)),
            })),
            ..Default::default()
        };

        {
            let state = state.clone();

            cx.open_window(options, move |window, cx| {
                let hwnd = tearoff_win::hwnd_of(window).unwrap_or(0);

                state.update(cx, |state, _| state.main_hwnd = hwnd);

                let docked_panel = cx.new(|_| PanelView {
                    dock: state.clone(),
                });

                cx.new(|cx| {
                    cx.observe(&state, |_, _, cx| cx.notify()).detach();

                    MainView {
                        state,
                        docked_panel,
                        started,
                        first_render_logged: false,
                    }
                })
            })
            .expect("не удалось открыть главное окно");
        }

        eprintln!("[timing] главное окно создано: {:?}", started.elapsed());

        // 3. Привязываем floating к главному (owner + размер по DPI).
        attach_floating(cx, &state);

        cx.activate(true);
    });
}

// Windows-specific native operations, kept beside the experiment entry point.
mod tearoff_win {
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
}
