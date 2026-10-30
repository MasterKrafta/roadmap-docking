//! Minimal experimental docking application entry point.

use dock_core::{Axis, DockCommand, DockMetrics, DockSide, Layout, Node, NodeId, PanelKey};
#[cfg(windows)]
use dock_core::{create_initial_layout, PanelConstraints, Profile, Size};
use gpui::{
    AnyElement, AppContext, Bounds, Context, IntoElement, Render, Subscription, WeakEntity, Window,
    WindowBounds, WindowControlArea, WindowHandle, WindowKind, WindowOptions, WindowVisibility, div,
    px, rgb, size,
};
use gpui::prelude::*;
#[cfg(windows)]
use gpui::{App, TitlebarOptions};
#[cfg(windows)]
use gpui_platform::application;
#[cfg(windows)]
use self::windows::NativeWindowHandle;

/// Starts the standalone docking experiment.
///
/// The experiment intentionally remains separate from the production
/// `dock-gpui` entry point while its architecture is being evaluated.
pub fn run() {
    #[cfg(windows)]
    run_windows();

    #[cfg(not(windows))]
    eprintln!("The docking experiment currently targets Windows.");
}

#[cfg(windows)]
fn run_windows() {
    application().run(|cx: &mut App| {
        let metrics = DockMetrics::default();
        let layout = match build_initial_layout(metrics) {
            Ok(layout) => layout,
            Err(error) => {
                eprintln!("Could not build the initial dock tree: {error}");
                return;
            }
        };

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            show: false,
            focus: false,
            titlebar: Some(TitlebarOptions {
                title: Some("Docking experiment".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let main_window = match cx.open_window(options, move |window, cx| {
            let main_window = cx.new(|_| MainWindow::new(layout, metrics));
            let weak_main_window = main_window.downgrade();

            window.on_window_should_close(cx, move |_window, app| {
                if let Err(error) = weak_main_window.update(app, |this, cx| {
                    this.close_floating_panels(cx);
                }) {
                    eprintln!("Could not close floating panels: {error:?}");
                }

                true
            });

            main_window
        }) {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("Could not open the experiment window: {error}");
                return;
            }
        };

        if let Err(error) = main_window.update(cx, |this, window, cx| {
            this.set_native_owner(window);
            this.open_startup_panels(cx);
        }) {
            eprintln!("Could not open the startup panels: {error:?}");
            return;
        }

        // All three windows have now completed GPUI's initial draw while hidden.
        // Show the main window first, then reveal the already-rendered panels.
        if let Err(error) = main_window.update(cx, |this, window, cx| {
            window.activate_window();
            this.show_floating_panels(cx);
        }) {
            eprintln!("Could not show the startup windows: {error:?}");
            return;
        }

        cx.activate(true);
    });
}

pub(crate) struct MainWindow {
    layout: Layout,
    metrics: DockMetrics,
    floating_panels: Vec<WindowHandle<FloatingPanel>>,
    panels_minimized_by_main: Vec<WindowHandle<FloatingPanel>>,
    visibility_subscription: Option<Subscription>,
    main_window_handle: Option<WindowHandle<MainWindow>>,
    #[cfg(windows)]
    native_owner: Option<NativeWindowHandle>,
}

#[derive(Clone, Copy)]
enum PanelKind {
    Layers,
    Properties,
}

impl PanelKind {
    const fn key(self) -> &'static str {
        match self {
            Self::Layers => "layers",
            Self::Properties => "properties",
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Layers => "Слои",
            Self::Properties => "Свойства",
        }
    }
}

impl MainWindow {
    #[cfg(windows)]
    pub(crate) fn set_native_owner(&mut self, window: &Window) {
        self.main_window_handle = Some(WindowHandle::new(window.window_handle().window_id()));
        self.native_owner = NativeWindowHandle::from_window(window);
        if self.native_owner.is_none() {
            eprintln!("Could not obtain the main window's native handle");
        }
    }

    #[cfg(windows)]
    fn configure_floating_window(
        &self,
        panel: &WindowHandle<FloatingPanel>,
        cx: &mut Context<Self>,
    ) {
        let Some(owner) = self.native_owner else {
            eprintln!("Could not assign a native owner to a floating panel");
            return;
        };

        match panel.update(cx, |_, window, _| {
            let Some(panel_handle) = NativeWindowHandle::from_window(window) else {
                return false;
            };

            windows::set_owner(panel_handle, owner);
            windows::remove_topmost(panel_handle)
        }) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("Could not remove topmost state from a floating panel");
            }
            Err(error) => {
                eprintln!("Could not configure a floating panel: {error:?}");
            }
        }
    }

    pub(crate) fn new(layout: Layout, metrics: DockMetrics) -> Self {
        Self {
            layout,
            metrics,
            floating_panels: Vec::new(),
            panels_minimized_by_main: Vec::new(),
            visibility_subscription: None,
            main_window_handle: None,
            #[cfg(windows)]
            native_owner: None,
        }
    }

    fn apply_layout_command(
        &mut self,
        command: DockCommand,
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        self.layout.apply(command, &self.metrics)?;
        cx.notify();
        Ok(())
    }

    /// Opens the panels that should be visible when the experiment starts.
    pub(crate) fn open_startup_panels(&mut self, cx: &mut Context<Self>) {
        self.open_floating_panel(PanelKind::Layers, false, cx);
        self.open_floating_panel(PanelKind::Properties, false, cx);
    }

    /// Shows the already-created panels after GPUI has rendered their first frame.
    pub(crate) fn show_floating_panels(&mut self, cx: &mut Context<Self>) {
        for panel in &self.floating_panels {
            if let Err(error) = panel.update(cx, |_, window, _| window.activate_window()) {
                eprintln!("Could not show a startup panel: {error:?}");
            }
        }
    }

    fn handle_main_window_visibility(
        &mut self,
        visibility: WindowVisibility,
        cx: &mut Context<Self>,
    ) {
        match visibility {
            WindowVisibility::Hidden => {
                self.panels_minimized_by_main.clear();

                for panel in &self.floating_panels {
                    match panel.update(cx, |_, window, _| {
                        if window.is_visible() {
                            window.minimize_window();
                            true
                        } else {
                            false
                        }
                    }) {
                        Ok(true) => self.panels_minimized_by_main.push(panel.clone()),
                        Ok(false) => {}
                        Err(_error) => {
                            // The panel may already have been closed by the user.
                        }
                    }
                }
            }
            WindowVisibility::Visible => {
                for panel in self.panels_minimized_by_main.drain(..) {
                    if let Err(error) =
                        panel.update(cx, |_, window, _| window.activate_window())
                    {
                        eprintln!("Could not restore a floating panel: {error:?}");
                    }
                }
            }
        }
    }

    fn open_floating_panel(
        &mut self,
        kind: PanelKind,
        show_immediately: bool,
        cx: &mut Context<Self>,
    ) {
        let key = match PanelKey::new(kind.key()) {
            Ok(key) => key,
            Err(error) => {
                eprintln!("Invalid built-in panel key: {error}");
                return;
            }
        };

        let bounds = WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(280.0), px(360.0)),
            cx,
        ));
        let options = WindowOptions {
            window_bounds: Some(bounds),
            // Configure ownership and z-order before making this window visible.
            show: false,
            focus: false,
            titlebar: None,
            kind: WindowKind::PopUp,
            ..Default::default()
        };

        let owner = cx.entity().downgrade();
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_| FloatingPanel { key, owner })
        }) {
            Ok(handle) => {
                #[cfg(windows)]
                self.configure_floating_window(&handle, cx);

                self.floating_panels.push(handle.clone());

                if show_immediately
                    && let Err(error) =
                        handle.update(cx, |_, window, _| window.activate_window())
                {
                    eprintln!("Could not show floating panel '{}': {error:?}", kind.key());
                }
            }
            Err(error) => {
                eprintln!("Could not open floating panel '{}': {error}", kind.key());
            }
        }
    }

    /// Closes every still-open panel before GPUI closes the main window.
    pub(crate) fn close_floating_panels(&mut self, cx: &mut Context<Self>) {
        self.panels_minimized_by_main.clear();

        for panel in self.floating_panels.drain(..) {
            match panel.update(cx, |_, window, _| window.remove_window()) {
                Ok(()) => {}
                Err(_error) => {
                    // If the user already closed this panel, cleanup is already complete.
                }
            }
        }
    }
}

fn menu_button(kind: PanelKind, owner: WeakEntity<MainWindow>) -> impl IntoElement {
    div()
        .id(kind.key())
        .px_3()
        .py_2()
        .bg(rgb(0x394451))
        .text_color(rgb(0xf0f2f5))
        .rounded_sm()
        .cursor_pointer()
        .child(kind.title())
        .on_click(move |_, _, cx| {
            if let Err(error) =
                owner.update(cx, |this, cx| this.open_floating_panel(kind, true, cx))
            {
                eprintln!("Could not access the main window to open a panel: {error:?}");
            }
        })
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.visibility_subscription.is_none() {
            self.visibility_subscription = Some(cx.observe_window_visibility(
                window,
                |this, visibility, _window, cx| {
                    this.handle_main_window_visibility(visibility, cx);
                },
            ));
        }

        let owner = cx.entity().downgrade();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x20252b))
            .text_color(rgb(0xe8ebef))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .h(px(48.0))
                    .px_3()
                    .bg(rgb(0x2b323a))
                    .border_b_1()
                    .border_color(rgb(0x414a55))
                    .child("Плавающие окна")
                    .child(menu_button(PanelKind::Layers, owner.clone()))
                    .child(menu_button(PanelKind::Properties, owner.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .w_full()
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .p_4()
                            .bg(rgb(0x20252b))
                            .text_color(rgb(0x626d7a))
                            .child("Рабочая область"),
                    )
                    .child(
                        self.layout
                            .main_area(DockSide::Right)
                            .map(|area| render_layout_node(&self.layout, area.root, owner.clone()))
                            .unwrap_or_else(|| {
                                div()
                                    .w(px(280.0))
                                    .h_full()
                                    .bg(rgb(0x262c33))
                                    .into_any_element()
                            }),
                    ),
            )
    }
}

struct FloatingPanel {
    key: PanelKey,
    owner: WeakEntity<MainWindow>,
}

impl Render for FloatingPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.key.as_str().to_owned();
        let owner = self.owner.clone();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0x292f37))
            .text_color(rgb(0xe8ebef))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .h(px(36.0))
                    .px_2()
                    .bg(rgb(0x2b323a))
                    .border_b_1()
                    .border_color(rgb(0x414a55))
                    .child(
                        div()
                            .id("panel-drag-area")
                            .flex()
                            .items_center()
                            .flex_1()
                            .h_full()
                            .px_2()
                            .window_control_area(WindowControlArea::Drag)
                            .child(title),
                    )
                    .child(
                        div()
                            .id("close-panel")
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .rounded_sm()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x596777)))
                            .occlude()
                            .on_click(move |_, window, cx| {
                                let panel_id = window.window_handle().window_id();
                                let main_window = owner.update(cx, |this, _cx| {
                                    this.floating_panels
                                        .retain(|panel| panel.window_id() != panel_id);
                                    this.panels_minimized_by_main
                                        .retain(|panel| panel.window_id() != panel_id);
                                    this.main_window_handle
                                });

                                match main_window {
                                    Ok(Some(main_window)) => {
                                        if let Err(error) = main_window.update(
                                            cx,
                                            |_, main_window, _| main_window.activate_window(),
                                        ) {
                                            eprintln!(
                                                "Could not activate the main window after closing a panel: {error:?}"
                                            );
                                        }
                                    }
                                    Ok(None) => {
                                        eprintln!(
                                            "Could not activate the main window after closing a panel: handle unavailable"
                                        );
                                    }
                                    Err(error) => {
                                        eprintln!(
                                            "Could not access the main window while closing a panel: {error:?}"
                                        );
                                    }
                                }

                                window.remove_window();
                            })
                            .child("×"),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .p_4()
                    .child(self.key.as_str().to_owned()),
            )
    }
}


#[cfg(windows)]
fn build_initial_layout(metrics: DockMetrics) -> Result<Layout, String> {
    let constraints = PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(280.0, 220.0),
        None,
        false,
    );
    let descriptors = ["layers", "color", "properties", "history"]
        .into_iter()
        .map(|key| {
            PanelKey::new(key)
                .map(|key| (key, constraints))
                .map_err(|error| format!("invalid built-in panel key '{key}': {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    create_initial_layout(Profile::Photoshop, &descriptors, metrics)
        .map_err(|error| error.to_string())
}

fn panel_title(key: &str) -> &str {
    match key {
        "layers" => "Слои",
        "color" => "Цвет",
        "properties" => "Свойства",
        "history" => "История",
        other => other,
    }
}

fn node_title(layout: &Layout, node_id: NodeId) -> String {
    match layout.node(node_id) {
        Some(Node::Panel(panel)) => layout
            .panel_instance(panel.panel_id)
            .map(|instance| panel_title(instance.key.as_str()).to_owned())
            .unwrap_or_else(|| "Неизвестная панель".to_owned()),
        Some(Node::Tabs(_)) => "Группа".to_owned(),
        Some(Node::Column(_)) => "Колонка".to_owned(),
        Some(Node::Split(_)) => "Разделение".to_owned(),
        None => "Неизвестный узел".to_owned(),
    }
}

fn render_layout_node(
    layout: &Layout,
    node_id: NodeId,
    owner: WeakEntity<MainWindow>,
) -> AnyElement {
    let Some(node) = layout.node(node_id) else {
        return div()
            .p_2()
            .text_color(rgb(0xf08a8a))
            .child("Узел дерева не найден")
            .into_any_element();
    };

    match node {
        Node::Column(column) => div()
            .flex()
            .flex_col()
            .w(px(column.width))
            .h_full()
            .gap_2()
            .p_2()
            .bg(rgb(0x262c33))
            .border_l_1()
            .border_color(rgb(0x414a55))
            .children(
                column
                    .groups
                    .iter()
                    .map(|child| render_layout_node(layout, *child, owner.clone())),
            )
            .into_any_element(),
        Node::Tabs(tabs) => {
            let tab_headers = tabs.children.iter().enumerate().map(|(index, child)| {
                let title = node_title(layout, *child);
                let active = index == tabs.active_index;
                let panel_id = match layout.node(*child) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                };

                let tab = div()
                    .id(format!("core-tab-{node_id:?}-{index}"))
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .bg(if active {
                        rgb(0x465260)
                    } else {
                        rgb(0x303841)
                    })
                    .text_color(if active {
                        rgb(0xf0f2f5)
                    } else {
                        rgb(0xaab4c0)
                    })
                    .child(title);

                if let Some(panel_id) = panel_id {
                    let activate_owner = owner.clone();
                    let close_owner = owner.clone();
                    let tab = tab
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x596777)))
                        .on_click(move |_, _, cx| {
                            match activate_owner.update(cx, |this, cx| {
                                this.apply_layout_command(
                                    DockCommand::ActivateTab {
                                        group: node_id,
                                        panel: panel_id,
                                    },
                                    cx,
                                )
                            }) {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    eprintln!("Could not activate dock tab: {error}");
                                }
                                Err(error) => {
                                    eprintln!("Could not access the dock layout: {error:?}");
                                }
                            }
                        });

                    let close_button = div()
                        .id(format!("close-core-tab-{node_id:?}-{index}"))
                        .px_1()
                        .rounded_sm()
                        .text_color(rgb(0xaab4c0))
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x596777)).text_color(rgb(0xffffff)))
                        .on_click(move |_, _, cx| {
                            match close_owner.update(cx, |this, cx| {
                                this.apply_layout_command(
                                    DockCommand::ClosePanel { panel: panel_id },
                                    cx,
                                )
                            }) {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    eprintln!("Could not close dock tab: {error}");
                                }
                                Err(error) => {
                                    eprintln!("Could not access the dock layout: {error:?}");
                                }
                            }
                        })
                        .child("×");

                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(tab)
                        .child(close_button)
                        .into_any_element()
                } else {
                    tab.into_any_element()
                }
            });

            let content = tabs
                .active_child()
                .map(|child| render_layout_node(layout, child, owner.clone()))
                .unwrap_or_else(|| {
                    div()
                        .flex_1()
                        .p_3()
                        .text_color(rgb(0xaab4c0))
                        .child("Пустая группа")
                        .into_any_element()
                });

            div()
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .bg(rgb(0x292f37))
                .border_b_1()
                .border_color(rgb(0x414a55))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .w_full()
                        .px_2()
                        .py_2()
                        .bg(rgb(0x2b323a))
                        .children(tab_headers),
                )
                .child(content)
                .into_any_element()
        }
        Node::Panel(panel) => {
            let title = layout
                .panel_instance(panel.panel_id)
                .map(|instance| panel_title(instance.key.as_str()))
                .unwrap_or("Неизвестная панель");

            div()
                .flex()
                .flex_col()
                .flex_1()
                .w_full()
                .p_3()
                .gap_2()
                .text_color(rgb(0xe8ebef))
                .child(div().text_color(rgb(0xf0f2f5)).child(title.to_owned()))
                .child(
                    div()
                        .text_color(rgb(0xaab4c0))
                        .child(format!("Содержимое панели «{title}»")),
                )
                .into_any_element()
        }
        Node::Split(split) => {
            let container = match split.axis {
                Axis::X => div().flex().flex_row(),
                Axis::Y => div().flex().flex_col(),
            };

            container
                .flex_1()
                .size_full()
                .gap_2()
                .children(
                    split
                        .children
                        .iter()
                        .map(|child| render_layout_node(layout, *child, owner.clone())),
                )
                .into_any_element()
        }
    }
}

#[cfg(windows)]
pub mod windows {
//! Windows-specific operations for GPUI-backed native windows.
//!
//! Call these functions on the UI thread and only while the source GPUI windows
//! remain alive. This module deliberately does not include the example's
//! high-resolution timer or its private GPUI redraw-message workaround.

use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use ::windows::Win32::{
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
}
