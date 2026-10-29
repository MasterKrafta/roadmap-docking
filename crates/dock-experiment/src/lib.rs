//! Minimal experimental docking application entry point.

use dock_core::PanelKey;
use gpui::{
    AppContext, Bounds, Context, IntoElement, Render, Subscription, WeakEntity, Window,
    WindowBounds, WindowControlArea, WindowHandle, WindowKind, WindowOptions, WindowVisibility, div,
    px, rgb, size,
};
use gpui::prelude::*;
#[cfg(windows)]
use gpui::{App, TitlebarOptions};
#[cfg(windows)]
use gpui_platform::application;
#[cfg(windows)]
use platform::windows::{self, NativeWindowHandle};

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

        let main_window = match cx.open_window(options, |window, cx| {
            let main_window = cx.new(|_| MainWindow::new());
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
    floating_panels: Vec<WindowHandle<FloatingPanel>>,
    panels_minimized_by_main: Vec<WindowHandle<FloatingPanel>>,
    visibility_subscription: Option<Subscription>,
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

    pub(crate) fn new() -> Self {
        Self {
            floating_panels: Vec::new(),
            panels_minimized_by_main: Vec::new(),
            visibility_subscription: None,
            #[cfg(windows)]
            native_owner: None,
        }
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

        match cx.open_window(options, move |_window, cx| {
            cx.new(|_| FloatingPanel { key })
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
                    .child("Панели")
                    .child(menu_button(PanelKind::Layers, owner.clone()))
                    .child(menu_button(PanelKind::Properties, owner)),
            )
            .child(div().flex_1().w_full().bg(rgb(0x20252b)))
    }
}

struct FloatingPanel {
    key: PanelKey,
}

impl Render for FloatingPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.key.as_str().to_owned();

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
                            .window_control_area(WindowControlArea::Close)
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

