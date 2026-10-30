//! Minimal experimental docking application entry point.

use dock_core::{
    Axis, DockCommand, DockMetrics, DockSide, DropTarget, FloatingGroupId, Layout, Node, NodeId,
    PanelId, Point, Rect, Size,
};
#[cfg(windows)]
use dock_core::{PanelConstraints, PanelKey, Profile};
use gpui::{
    AnyElement, AppContext, Bounds, Context, Entity, IntoElement, Pixels, Render, Subscription,
    WeakEntity,
    Window, WindowBounds, WindowControlArea, WindowHandle, WindowKind, WindowOptions,
    WindowVisibility, div, px, rgb, size,
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
        let layout = cx.new(|_| layout);

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
    layout: Entity<Layout>,
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

#[derive(Clone, Copy)]
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
        layout: Entity<Layout>,
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
        self.layout.update(cx, |layout, layout_cx| {
            layout.apply(command, &self.metrics)?;
            layout_cx.notify();
            Ok(())
        })
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
        self.layout.update(cx, |layout, layout_cx| {
            layout.apply(
                DockCommand::ResizeFloatingBounds { group, bounds },
                &self.metrics,
            )?;
            layout_cx.notify();
            Ok(())
        })?;

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
            .copied()
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

        let floating_group_id = {
            let layout = self.layout.read(cx);
            floating_group_for_panel(&layout, binding.panel_id)
        };

        let Some(floating_group_id) = floating_group_id else {
            let docked_group = {
                let layout = self.layout.read(cx);
                main_tabs_group_for_panel(&layout, binding.panel_id)
            };

            if let Some(group) = docked_group {
                if let Err(error) = self.apply_layout_command(
                    DockCommand::ActivateTab {
                        group,
                        panel: binding.panel_id,
                    },
                    cx,
                ) {
                    eprintln!("Could not activate docked panel '{}': {error}", kind.key());
                }
            } else {
                eprintln!("Panel '{}' is not present in the dock layout", kind.key());
            }
            return;
        };

        if let Err(error) = self.apply_layout_command(
            DockCommand::RaiseFloating {
                group: floating_group_id,
            },
            cx,
        ) {
            eprintln!("Could not raise floating panel '{}': {error}", kind.key());
            return;
        }

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(binding.window_bounds)),
            // Configure ownership and z-order before making this window visible.
            show: false,
            focus: false,
            titlebar: None,
            kind: WindowKind::PopUp,
            ..Default::default()
        };

        let owner = cx.entity().downgrade();
        let layout = self.layout.clone();
        let panel_id = binding.panel_id;
        match cx.open_window(options, move |_window, cx| {
            cx.new(|_| FloatingPanel {
                panel_id,
                floating_group_id,
                owner,
                layout,
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

    fn dock_panel_to_side(
        &mut self,
        panel_id: PanelId,
        side: DockSide,
        cx: &mut Context<Self>,
    ) -> Result<Option<WindowHandle<MainWindow>>, dock_core::CommandError> {
        let index = {
            let layout = self.layout.read(cx);
            if floating_group_for_panel(&layout, panel_id).is_none() {
                return Err(dock_core::CommandError::InvalidTarget {
                    reason: "only a floating panel can be docked from its floating window",
                });
            }

            let column_count = main_column_count(&layout, side);
            match side {
                DockSide::Left => 0,
                DockSide::Right => column_count,
            }
        };

        self.apply_layout_command(
            DockCommand::MovePanel {
                panel: panel_id,
                target: DropTarget::MainColumn { side, index },
            },
            cx,
        )?;

        let window_id = self
            .floating_panels
            .iter()
            .find(|panel| panel.panel_id == panel_id)
            .map(|panel| panel.handle.window_id());

        self.floating_panels
            .retain(|panel| panel.panel_id != panel_id);

        if let Some(window_id) = window_id {
            self.panels_minimized_by_main
                .retain(|panel| panel.window_id() != window_id);
        }

        Ok(self.main_window_handle)
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
        let layout = self.layout.read(cx);
        let left_dock = render_main_dock(
            &layout,
            DockSide::Left,
            self.metrics,
            owner.clone(),
        );
        let right_dock = render_main_dock(
            &layout,
            DockSide::Right,
            self.metrics,
            owner.clone(),
        );

        let mut workspace = div()
            .flex()
            .flex_row()
            .flex_1()
            .size_full()
            .bg(rgb(0x20252b));

        if let Some(left_dock) = left_dock {
            workspace = workspace.child(left_dock);
        }

        workspace = workspace.child(
            div()
                .flex_1()
                .size_full()
                .bg(rgb(0x20252b)),
        );

        if let Some(right_dock) = right_dock {
            workspace = workspace.child(right_dock);
        }

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
                    .child(menu_button(PanelKind::Properties, owner.clone())),
            )
            .child(workspace)
    }
}

struct FloatingPanel {
    panel_id: PanelId,
    floating_group_id: FloatingGroupId,
    owner: WeakEntity<MainWindow>,
    layout: Entity<Layout>,
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

        let layout = self.layout.read(cx);
        let title = layout
            .panel_instance(self.panel_id)
            .map(|instance| panel_title(instance.key.as_str()).to_owned())
            .unwrap_or_else(|| "Панель".to_owned());
        let content = render_floating_group(
            &layout,
            self.floating_group_id,
            self.owner.clone(),
        )
        .unwrap_or_else(|| {
            div()
                .flex_1()
                .p_4()
                .text_color(rgb(0xf08a8a))
                .child("Дерево панели не найдено")
                .into_any_element()
        });

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
                    .child(dock_panel_button(
                        "←",
                        "dock-panel-left",
                        DockSide::Left,
                        owner.clone(),
                        panel_id,
                    ))
                    .child(dock_panel_button(
                        "→",
                        "dock-panel-right",
                        DockSide::Right,
                        owner.clone(),
                        panel_id,
                    ))
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
            .child(content)
    }
}

fn dock_panel_button(
    label: &'static str,
    id: &'static str,
    side: DockSide,
    owner: WeakEntity<MainWindow>,
    panel_id: PanelId,
) -> impl IntoElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.0))
        .rounded_sm()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x596777)))
        .occlude()
        .on_click(move |_, window, cx| {
            match owner.update(cx, |main, cx| main.dock_panel_to_side(panel_id, side, cx)) {
                Ok(Ok(main_window)) => {
                    if let Some(main_window) = main_window
                        && let Err(error) = main_window.update(cx, |_, window, _| {
                            window.activate_window();
                        })
                    {
                        eprintln!("Could not activate the main window after docking: {error:?}");
                    }
                    window.remove_window();
                }
                Ok(Err(error)) => {
                    eprintln!("Could not dock panel to {:?}: {error}", side);
                }
                Err(error) => {
                    eprintln!("Could not access the main window while docking: {error:?}");
                }
            }
        })
        .child(label)
}

fn node_contains_panel(layout: &Layout, node_id: NodeId, panel_id: PanelId) -> bool {
    match layout.node(node_id) {
        Some(Node::Panel(panel)) => panel.panel_id == panel_id,
        Some(Node::Column(column)) => column
            .groups
            .iter()
            .any(|child| node_contains_panel(layout, *child, panel_id)),
        Some(Node::Tabs(tabs)) => tabs
            .children
            .iter()
            .any(|child| node_contains_panel(layout, *child, panel_id)),
        Some(Node::Split(split)) => split
            .children
            .iter()
            .any(|child| node_contains_panel(layout, *child, panel_id)),
        None => false,
    }
}

fn floating_group_for_panel(layout: &Layout, panel_id: PanelId) -> Option<FloatingGroupId> {
    layout.floating_groups().find_map(|(group_id, floating)| {
        let root = layout.dock_area(floating.area)?.root;
        node_contains_panel(layout, root, panel_id).then_some(group_id)
    })
}

fn tabs_group_for_panel(
    layout: &Layout,
    node_id: NodeId,
    panel_id: PanelId,
) -> Option<NodeId> {
    match layout.node(node_id)? {
        Node::Tabs(tabs) => tabs
            .children
            .iter()
            .any(|child| node_contains_panel(layout, *child, panel_id))
            .then_some(node_id),
        Node::Column(column) => column
            .groups
            .iter()
            .find_map(|child| tabs_group_for_panel(layout, *child, panel_id)),
        Node::Split(split) => split
            .children
            .iter()
            .find_map(|child| tabs_group_for_panel(layout, *child, panel_id)),
        Node::Panel(_) => None,
    }
}

fn main_tabs_group_for_panel(layout: &Layout, panel_id: PanelId) -> Option<NodeId> {
    [DockSide::Left, DockSide::Right]
        .into_iter()
        .filter_map(|side| layout.main_area(side).map(|area| area.root))
        .find_map(|root| tabs_group_for_panel(layout, root, panel_id))
}

fn main_column_count(layout: &Layout, side: DockSide) -> usize {
    let Some(area) = layout.main_area(side) else {
        return 0;
    };

    match layout.node(area.root) {
        Some(Node::Column(_)) => 1,
        Some(Node::Split(split)) if split.axis == Axis::X => split.children.len(),
        _ => 0,
    }
}

fn main_area_width(layout: &Layout, side: DockSide, metrics: DockMetrics) -> Option<f32> {
    let area = layout.main_area(side)?;
    match layout.node(area.root)? {
        Node::Column(column) => Some(column.width),
        Node::Split(split) if split.axis == Axis::X => {
            let mut width = 0.0;
            for child in &split.children {
                let Some(Node::Column(column)) = layout.node(*child) else {
                    return None;
                };
                width += column.width;
            }
            width += metrics.splitter_thickness * split.children.len().saturating_sub(1) as f32;
            Some(width)
        }
        _ => None,
    }
}

fn render_main_dock(
    layout: &Layout,
    side: DockSide,
    metrics: DockMetrics,
    owner: WeakEntity<MainWindow>,
) -> Option<AnyElement> {
    let root = layout.main_area(side)?.root;
    let width = main_area_width(layout, side, metrics).unwrap_or(metrics.default_column_width);
    let content = render_core_node(layout, root, owner);

    let dock = div()
        .flex()
        .flex_col()
        .h_full()
        .w(px(width))
        .bg(rgb(0x262c33));

    let dock = match side {
        DockSide::Left => dock.border_r_1().border_color(rgb(0x414a55)),
        DockSide::Right => dock.border_l_1().border_color(rgb(0x414a55)),
    };

    Some(dock.child(content).into_any_element())
}

fn render_floating_group(
    layout: &Layout,
    group: FloatingGroupId,
    owner: WeakEntity<MainWindow>,
) -> Option<AnyElement> {
    let root = layout
        .floating_group(group)
        .and_then(|floating| layout.dock_area(floating.area))
        .map(|area| area.root)?;

    Some(render_core_node(layout, root, owner))
}

fn render_core_node(
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
            .flex_1()
            .w_full()
            .gap_2()
            .p_2()
            .bg(rgb(0x262c33))
            .children(
                column
                    .groups
                    .iter()
                    .map(|child| render_core_node(layout, *child, owner.clone())),
            )
            .into_any_element(),
        Node::Tabs(tabs) => {
            let tab_headers = tabs.children.iter().enumerate().map(|(index, child)| {
                let title = core_node_title(layout, *child);
                let active = index == tabs.active_index;
                let panel_id = match layout.node(*child) {
                    Some(Node::Panel(panel)) => Some(panel.panel_id),
                    _ => None,
                };

                let mut tab = div()
                    .id(format!("core-tab-{node_id:?}-{index}"))
                    .px_3()
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
                    let tab_owner = owner.clone();
                    tab = tab
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0x596777)))
                        .on_click(move |_, _, cx| {
                            match tab_owner.update(cx, |this, cx| {
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
                }

                tab
            });

            let header = div()
                .flex()
                .items_center()
                .gap_1()
                .w_full()
                .px_2()
                .py_2()
                .bg(rgb(0x2b323a))
                .children(tab_headers);

            let content = tabs
                .active_child()
                .map(|child| render_core_node(layout, child, owner.clone()))
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
                .child(header)
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
                .text_color(rgb(0xe8ebef))
                .child(title.to_owned())
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
                        .map(|child| render_core_node(layout, *child, owner.clone())),
                )
                .into_any_element()
        }
    }
}

fn core_node_title(layout: &Layout, node_id: NodeId) -> String {
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

#[cfg(windows)]
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
