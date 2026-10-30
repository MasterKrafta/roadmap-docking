//! Minimal experimental docking application entry point.

use dock_core::{
    DockCommand, DockMetrics, DockSide, DropTarget, FloatingGroupId, Layout, Node, PanelConstraints,
    PanelId, PanelKey, Point, Profile, Rect, Size,
};
use gpui::{
    AppContext, Bounds, Context, IntoElement, Pixels, Render, Subscription, WeakEntity, Window,
    WindowBounds, WindowControlArea, WindowHandle, WindowKind, WindowOptions, WindowVisibility, div,
    px, rgb, size,
};
use gpui::prelude::*;
#[cfg(windows)]
use gpui::{App, TitlebarOptions};
#[cfg(windows)]
use gpui_platform::application;
#[cfg(windows)]
use platform::windows::NativeWindowHandle;

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
        let panel_bounds = [
            Bounds::centered(None, size(px(280.0), px(360.0)), cx),
            Bounds::centered(None, size(px(280.0), px(360.0)), cx),
        ];
        let (layout, panel_bindings) = match build_initial_layout(metrics, panel_bounds) {
            Ok(result) => result,
            Err(error) => {
                eprintln!("Could not build the initial dock trees: {error}");
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
            let main_window =
                cx.new(|_| MainWindow::new(layout, metrics, panel_bindings));
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
    panel_bindings: [PanelBinding; 2],
    floating_panels: Vec<FloatingPanelWindow>,
    panels_minimized_by_main: Vec<WindowHandle<FloatingPanel>>,
    visibility_subscription: Option<Subscription>,
    main_window_handle: Option<WindowHandle<MainWindow>>,
    #[cfg(windows)]
    native_owner: Option<NativeWindowHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PanelKind {
    Layers,
    Properties,
}

#[derive(Clone)]
struct PanelBinding {
    kind: PanelKind,
    panel_id: PanelId,
    floating_group_id: FloatingGroupId,
    window_bounds: Bounds<Pixels>,
}

struct FloatingPanelWindow {
    panel_id: PanelId,
    handle: WindowHandle<FloatingPanel>,
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

            platform::windows::set_owner(panel_handle, owner);
            platform::windows::remove_topmost(panel_handle)
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

    pub(crate) fn new(
        layout: Layout,
        metrics: DockMetrics,
        panel_bindings: [PanelBinding; 2],
    ) -> Self {
        Self {
            layout,
            metrics,
            panel_bindings,
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
            if let Err(error) = panel
                .handle
                .update(cx, |_, window, _| window.activate_window())
            {
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
                    match panel.handle.update(cx, |_, window, _| {
                        if window.is_visible() {
                            window.minimize_window();
                            true
                        } else {
                            false
                        }
                    }) {
                        Ok(true) => self
                            .panels_minimized_by_main
                            .push(panel.handle.clone()),
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

    fn sync_floating_panel_bounds(
        &mut self,
        group: FloatingGroupId,
        native_bounds: Bounds<Pixels>,
        bounds: Rect,
        cx: &mut Context<Self>,
    ) -> Result<(), dock_core::CommandError> {
        self.layout.apply(
            DockCommand::ResizeFloatingBounds { group, bounds },
            &self.metrics,
        )?;

        if let Some(binding) = self
            .panel_bindings
            .iter_mut()
            .find(|binding| binding.floating_group_id == group)
        {
            binding.window_bounds = native_bounds;
        }

        cx.notify();
        Ok(())
    }

    fn open_floating_panel(
        &mut self,
        kind: PanelKind,
        show_immediately: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(binding) = self
            .panel_bindings
            .iter()
            .find(|binding| binding.kind == kind)
            .cloned()
        else {
            eprintln!("No dock-core tree is registered for panel '{}'", kind.key());
            return;
        };

        if let Some(existing) = self
            .floating_panels
            .iter()
            .find(|panel| panel.panel_id == binding.panel_id)
        {
            if show_immediately
                && let Err(error) = existing
                    .handle
                    .update(cx, |_, window, _| window.activate_window())
            {
                eprintln!("Could not activate floating panel '{}': {error:?}", kind.key());
            }
            return;
        }

        if let Err(error) = self.apply_layout_command(
            DockCommand::RaiseFloating {
                group: binding.floating_group_id,
            },
            cx,
        ) {
            eprintln!("Could not raise floating panel '{}': {error}", kind.key());
            return;
        }

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(binding.window_bounds.clone())),
            // Configure ownership and z-order before making this window visible.
            show: false,
            focus: false,
            titlebar: None,
            kind: WindowKind::PopUp,
            ..Default::default()
        };

        let owner = cx.entity().downgrade();
        let panel_id = binding.panel_id;
        let floating_group_id = binding.floating_group_id;
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_| FloatingPanel {
                panel_id,
                floating_group_id,
                owner,
                bounds_subscription: None,
            })
        }) {
            Ok(handle) => {
                #[cfg(windows)]
                self.configure_floating_window(&handle, cx);

                self.floating_panels.push(FloatingPanelWindow {
                    panel_id,
                    handle: handle.clone(),
                });

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
            match panel
                .handle
                .update(cx, |_, window, _| window.remove_window())
            {
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
                    .flex_1()
                    .size_full()
                    .bg(rgb(0x20252b)),
            )
    }
}

struct FloatingPanel {
    panel_id: PanelId,
    floating_group_id: FloatingGroupId,
    owner: WeakEntity<MainWindow>,
    bounds_subscription: Option<Subscription>,
}

impl FloatingPanel {
    fn sync_native_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let native_bounds = window.window_bounds().get_bounds();
        let bounds = core_bounds(&native_bounds);
        let group = self.floating_group_id;

        match self.owner.update(cx, |main, cx| {
            main.sync_floating_panel_bounds(group, native_bounds, bounds, cx)
        }) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                eprintln!("Could not synchronize floating panel bounds: {error}");
            }
            Err(error) => {
                eprintln!("Could not access the dock layout for floating bounds: {error:?}");
            }
        }
    }
}

impl Render for FloatingPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.bounds_subscription.is_none() {
            self.bounds_subscription =
                Some(cx.observe_window_bounds(window, |this, window, cx| {
                    this.sync_native_bounds(window, cx);
                }));
        }

        let title = self
            .owner
            .upgrade()
            .and_then(|owner| {
                owner
                    .read(cx)
                    .layout
                    .panel_instance(self.panel_id)
                    .map(|instance| panel_title(instance.key.as_str()).to_owned())
            })
            .unwrap_or_else(|| "Панель".to_owned());

        let owner = self.owner.clone();
        let panel_id = self.panel_id;

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
                            .child(title.clone()),
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
                                let window_id = window.window_handle().window_id();
                                let main_window = owner.update(cx, |this, _cx| {
                                    this.floating_panels
                                        .retain(|panel| panel.panel_id != panel_id);
                                    this.panels_minimized_by_main
                                        .retain(|panel| panel.window_id() != window_id);
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
            .child(div().flex_1().w_full().p_4().child(title))
    }
}


#[cfg(windows)]
fn build_initial_layout(
    metrics: DockMetrics,
    window_bounds: [Bounds<Pixels>; 2],
) -> Result<(Layout, [PanelBinding; 2]), String> {
    let [layers_bounds, properties_bounds] = window_bounds;
    let mut layout = Layout::new(Profile::Generic);

    let layers = create_floating_panel_tree(
        &mut layout,
        PanelKind::Layers,
        DockSide::Left,
        layers_bounds,
        &metrics,
    )?;
    let properties = create_floating_panel_tree(
        &mut layout,
        PanelKind::Properties,
        DockSide::Right,
        properties_bounds,
        &metrics,
    )?;

    layout
        .validate(&metrics)
        .map_err(|errors| format!("dock-core rejected the initial panel trees: {errors:?}"))?;

    Ok((layout, [layers, properties]))
}

#[cfg(windows)]
fn create_floating_panel_tree(
    layout: &mut Layout,
    kind: PanelKind,
    side: DockSide,
    window_bounds: Bounds<Pixels>,
    metrics: &DockMetrics,
) -> Result<PanelBinding, String> {
    let key = PanelKey::new(kind.key())
        .map_err(|error| format!("invalid panel key '{}': {error}", kind.key()))?;

    layout
        .apply(
            DockCommand::OpenPanel {
                key: key.clone(),
                constraints: panel_constraints(),
                target: Some(DropTarget::MainColumn { side, index: 0 }),
            },
            metrics,
        )
        .map_err(|error| format!("could not create panel '{}': {error}", kind.key()))?;

    let root = layout
        .main_area(side)
        .map(|area| area.root)
        .ok_or_else(|| format!("dock-core did not create the {:?} panel area", side))?;
    let panel_id = layout
        .panel_instances()
        .find_map(|(panel_id, instance)| (instance.key == key).then_some(panel_id))
        .ok_or_else(|| format!("dock-core did not register panel '{}'", kind.key()))?;

    layout
        .apply(
            DockCommand::Float {
                node: root,
                bounds: core_bounds(&window_bounds),
            },
            metrics,
        )
        .map_err(|error| format!("could not float panel '{}': {error}", kind.key()))?;

    let floating_group_id = layout
        .floating_groups()
        .find_map(|(group_id, floating)| {
            layout
                .dock_area(floating.area)
                .is_some_and(|area| area.root == root)
                .then_some(group_id)
        })
        .ok_or_else(|| format!("dock-core did not create a floating group for '{}'", kind.key()))?;

    let is_expected_tree = match layout.node(root) {
        Some(Node::Column(column)) if column.groups.len() == 1 => {
            match layout.node(column.groups[0]) {
                Some(Node::Tabs(tabs)) if tabs.children.len() == 1 => {
                    matches!(
                        layout.node(tabs.children[0]),
                        Some(Node::Panel(panel)) if panel.panel_id == panel_id
                    )
                }
                _ => false,
            }
        }
        _ => false,
    };
    if !is_expected_tree {
        return Err(format!(
            "dock-core created an unexpected tree for '{}'; expected Column -> Tabs -> Panel",
            kind.key()
        ));
    }

    Ok(PanelBinding {
        kind,
        panel_id,
        floating_group_id,
        window_bounds,
    })
}

fn core_bounds(bounds: &Bounds<Pixels>) -> Rect {
    Rect::new(
        Point::new(bounds.origin.x.as_f32(), bounds.origin.y.as_f32()),
        Size::new(
            bounds.size.width.as_f32(),
            bounds.size.height.as_f32(),
        ),
    )
}

fn panel_constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(280.0, 220.0),
        None,
        true,
    )
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
