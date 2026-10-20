use dock_core::{
    DockCommand, DockMetrics, DockSide, DragCancelReason, DragSession, DragSessionState,
    DropPayload, DropTarget, DropZone, FloatSource, FloatingGroupId, Layout, LayoutRects, Node,
    NodeId, PanelId, PanelKey, Point as DockPoint, Rect as DockRect, ResizeEdge,
    Size as DockSize, SplitterKind, SplitterRect, compute_rects, resolve_drop,
};
use dock_gpui::{DockGroupData, TabItem, dock_column_view, drop_overlay};
use gpui::{
    AnyElement, App, Context, FocusHandle, KeyDownEvent, MouseButton, MouseMoveEvent, MouseUpEvent,
    Pixels, Point as GpuiPoint, Render, Window, div, prelude::*, px, rgb,
};
use ui::Theme;

use crate::panels::{PanelRegistry, PlaceholderPanelContent};

/// The single main application window. Its tree receives one shared application Theme;
/// child builders borrow that theme rather than constructing their own.
pub struct MainWindow {
    theme: Theme,
    metrics: DockMetrics,
    registry: PanelRegistry,
    layout: Layout,
    focus_handle: FocusHandle,
    drag_session: DragSession,
    floating_interaction: Option<FloatingInteraction>,
    floating_drag: Option<FloatingDragState>,
    window_menu_open: bool,
}

struct WindowGeometry {
    menu_height: f32,
    panel_menu_height: f32,
    options_height: f32,
    body_height: f32,
    body_top: f32,
    docking_viewport_width: f32,
    rects: LayoutRects,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum FloatingInteraction {
    Resize {
        floating: FloatingGroupId,
        edge: ResizeEdge,
        pointer_origin: DockPoint,
        initial_bounds: DockRect,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FloatingDragState {
    floating: FloatingGroupId,
    pointer_origin: DockPoint,
    initial_bounds: DockRect,
    current_bounds: DockRect,
}
impl MainWindow {
    pub fn new(
        theme: Theme,
        metrics: DockMetrics,
        registry: PanelRegistry,
        layout: Layout,
        focus_handle: FocusHandle,
    ) -> Self {
        Self {
            theme,
            metrics,
            registry,
            layout,
            focus_handle,
            drag_session: DragSession::new(),
            floating_interaction: None,
            floating_drag: None,
            window_menu_open: false,
        }
    }

    fn window_geometry(&self, window: &Window) -> WindowGeometry {
        let bounds = window.bounds();
        let window_width = bounds.size.width.as_f32().max(0.0);
        let window_height = bounds.size.height.as_f32().max(0.0);
        let metrics = self.theme.metrics;
        let menu_height = metrics.column_header_height.min(window_height);
        let panel_menu_height = if self.window_menu_open {
            (metrics.tab_strip_height.max(24.0) * (self.registry.len() as f32 + 1.0)
                + metrics.content_padding
                + 2.0)
                .min((window_height - menu_height - metrics.tab_strip_height).max(0.0))
        } else {
            0.0
        };
        let options_height = metrics
            .tab_strip_height
            .min((window_height - menu_height - panel_menu_height).max(0.0));
        let body_height =
            (window_height - menu_height - panel_menu_height - options_height).max(0.0);
        let docking_viewport_width = (window_width - metrics.rail_width).max(0.0);
        let mut rects = compute_rects(
            &self.layout,
            DockSize::new(docking_viewport_width, body_height),
            &self.metrics,
        );
        align_tab_header_rects(
            &self.layout,
            &mut rects,
            metrics.content_padding,
            metrics.button_size,
        );
        self.apply_floating_drag_preview(&mut rects);

        WindowGeometry {
            menu_height,
            panel_menu_height,
            options_height,
            body_height,
            body_top: menu_height + panel_menu_height + options_height,
            docking_viewport_width,
            rects,
        }
    }

    fn local_dock_point(
        &self,
        position: GpuiPoint<Pixels>,
        geometry: &WindowGeometry,
    ) -> DockPoint {
        DockPoint::new(
            position.x.as_f32() - self.theme.metrics.rail_width,
            position.y.as_f32() - geometry.body_top,
        )
    }

    fn begin_drag(
        &mut self,
        payload: DropPayload,
        position: GpuiPoint<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            self.drag_session.state(),
            DragSessionState::Pressed { .. } | DragSessionState::Dragging { .. }
        ) {
            return;
        }

        self.focus_handle.focus(window, cx);
        let geometry = self.window_geometry(window);
        let origin = self.local_dock_point(position, &geometry);
        if let DropPayload::Panel(panel) = payload {
            if let Some(floating) = self
                .layout
                .floating_groups()
                .iter()
                .find_map(|(floating, group)| group.panels.contains(&panel).then_some(floating))
            {
                if let Err(error) = self.layout.apply(
                    DockCommand::RaiseFloating { floating },
                    &self.metrics,
                ) {
                    eprintln!("Could not raise floating group {floating:?} for tab drag: {error}");
                    return;
                }
            }
        }

        if self.drag_session.press(payload, origin) {
            cx.notify();
        }
    }

    fn cancel_panel_drag(&mut self, cx: &mut Context<Self>) {
        if cancel_drag_interaction(&mut self.drag_session, &mut self.floating_drag) {
            cx.notify();
        }
    }

    fn begin_floating_interaction(
        &mut self,
        floating: FloatingGroupId,
        edge: Option<ResizeEdge>,
        position: GpuiPoint<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            self.drag_session.state(),
            DragSessionState::Pressed { .. } | DragSessionState::Dragging { .. }
        ) || self.floating_interaction.is_some()
        {
            return;
        }

        let geometry = self.window_geometry(window);
        let Some(initial_bounds) = geometry
            .rects
            .floating_groups
            .iter()
            .find(|rect| rect.floating == floating)
            .map(|rect| rect.bounds)
        else {
            return;
        };
        let pointer_origin = self.local_dock_point(position, &geometry);
        // The title bar is also the visible surface behind the top resize hit band.
        // If hit-testing lands on the title bar instead of its overlapping resize child,
        // route the gesture by its exact pointer position so the top edge/corners resize
        // rather than accidentally starting a move.
        let edge = edge.or_else(|| {
            titlebar_resize_edge(
                pointer_origin,
                initial_bounds,
                self.metrics.splitter_hit_zone,
            )
        });

        if let Err(error) = self.layout.apply(
            DockCommand::RaiseFloating { floating },
            &self.metrics,
        ) {
            eprintln!("Could not raise floating group {floating:?}: {error}");
            return;
        }

        match edge {
            Some(edge) => {
                self.floating_drag = None;
                self.floating_interaction = Some(FloatingInteraction::Resize {
                    floating,
                    edge,
                    pointer_origin,
                    initial_bounds,
                });
            }
            None => {
                self.floating_interaction = None;
                if self
                    .drag_session
                    .press(DropPayload::FloatingGroup(floating), pointer_origin)
                {
                    self.floating_drag = Some(FloatingDragState {
                        floating,
                        pointer_origin,
                        initial_bounds,
                        current_bounds: initial_bounds,
                    });
                } else {
                    self.floating_drag = None;
                }
            }
        }
        cx.notify();
    }

    fn focus_floating(&mut self, floating: FloatingGroupId, cx: &mut Context<Self>) {
        match self.layout.apply(DockCommand::RaiseFloating { floating }, &self.metrics) {
            Ok(_) => cx.notify(),
            Err(error) => eprintln!("Could not raise floating group {floating:?}: {error}"),
        }
    }

    fn floating_minimum_size(&self, floating: FloatingGroupId) -> DockSize {
        let mut minimum_width = self
            .metrics
            .default_column_width
            .max(self.metrics.min_column_width);
        let mut minimum_content_height = self.metrics.min_group_height;
        if let Some(group) = self.layout.floating_groups().get(floating) {
            for panel_id in &group.panels {
                if let Some(panel) = self.layout.panels().get(*panel_id) {
                    minimum_width = minimum_width.max(panel.constraints().min_size().width);
                    minimum_content_height =
                        minimum_content_height.max(panel.constraints().min_size().height);
                }
            }
        }
        DockSize::new(
            minimum_width,
            minimum_content_height
                + self.metrics.column_header_height
                + self.metrics.tab_strip_height,
        )
    }

    fn update_floating_interaction(
        &mut self,
        position: GpuiPoint<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(FloatingInteraction::Resize {
            floating,
            edge,
            pointer_origin,
            initial_bounds,
        }) = self.floating_interaction
        else {
            return;
        };
        let geometry = self.window_geometry(window);
        let point = self.local_dock_point(position, &geometry);
        let viewport = DockSize::new(
            geometry.docking_viewport_width.max(0.0),
            geometry.body_height.max(0.0),
        );
        let delta = DockPoint::new(
            point.x - pointer_origin.x,
            point.y - pointer_origin.y,
        );
        let minimum = self.floating_minimum_size(floating);
        let rect = resize_floating_rect(initial_bounds, edge, delta, minimum, viewport);

        let current_bounds = self
            .layout
            .floating_groups()
            .get(floating)
            .map(|group| group.bounds);
        if current_bounds == Some(rect) {
            return;
        }

        if let Err(error) = self.layout.apply(
            DockCommand::ResizeFloating { floating, rect },
            &self.metrics,
        ) {
            eprintln!("Could not resize floating group {floating:?}: {error}");
        } else {
            cx.notify();
        }
    }

    fn update_panel_drag(
        &mut self,
        position: GpuiPoint<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let payload = match self.drag_session.state() {
            DragSessionState::Pressed { payload, .. }
            | DragSessionState::Dragging { payload, .. } => Some(*payload),
            DragSessionState::Idle
            | DragSessionState::Dropped { .. }
            | DragSessionState::Cancelled { .. } => None,
        };
        let Some(payload) = payload else {
            return;
        };

        let base_geometry = self.window_geometry(window);
        let point = self.local_dock_point(position, &base_geometry);
        let previous_state = self.drag_session.state().clone();

        // Enter/update Dragging before resolving so the floating group can follow the
        // cursor as a visual preview, while the actual Layout remains untouched.
        self.drag_session.pointer_moved(point, None);
        let dragging_payload = match self.drag_session.state() {
            DragSessionState::Dragging { payload, .. } => Some(*payload),
            _ => None,
        };

        let target = if let Some(dragging_payload) = dragging_payload {
            if let (
                DropPayload::FloatingGroup(floating),
                Some(drag),
            ) = (dragging_payload, self.floating_drag)
                && drag.floating == floating
            {
                let viewport = DockSize::new(
                    base_geometry.docking_viewport_width.max(0.0),
                    base_geometry.body_height.max(0.0),
                );
                let current_bounds = floating_drag_preview_bounds(
                    drag.initial_bounds,
                    drag.pointer_origin,
                    point,
                    viewport,
                );
                if let Some(state) = self.floating_drag.as_mut() {
                    state.current_bounds = current_bounds;
                }
            }

            let geometry = self.window_geometry(window);
            let target_rects = drop_target_rects(&geometry.rects, dragging_payload);
            let target = resolve_drop(
                &self.layout,
                point,
                dragging_payload,
                &target_rects,
                &self.metrics,
            );
            // The capture handle detaches a docked group only. Floating groups can
            // dock into groups/split zones or merge with another floating group.
            match dragging_payload {
                DropPayload::Group(_) => {
                    target.filter(|target| matches!(target, DropTarget::FreeArea { .. }))
                }
                DropPayload::Panel(_) | DropPayload::FloatingGroup(_) => target,
            }
        } else {
            None
        };

        self.drag_session.pointer_moved(point, target);
        if self.drag_session.state() != &previous_state {
            cx.notify();
        }
    }

    fn finish_panel_drag(
        &mut self,
        position: GpuiPoint<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(
            self.drag_session.state(),
            DragSessionState::Pressed { .. } | DragSessionState::Dragging { .. }
        ) {
            return;
        }

        // Resolve once more on release to handle a coalesced final pointer event.
        self.update_panel_drag(position, window, cx);
        let was_dragging = matches!(self.drag_session.state(), DragSessionState::Dragging { .. });
        let request = self.drag_session.release();

        if let Some(request) = request {
            let supported_target = match request.payload {
                DropPayload::Panel(_) => matches!(
                    &request.target,
                    DropTarget::Group {
                        zone: DropZone::Center | DropZone::Top | DropZone::Bottom,
                        ..
                    } | DropTarget::TabInsertion { .. }
                        | DropTarget::Column {
                            zone: DropZone::Top
                                | DropZone::Bottom
                                | DropZone::Left
                                | DropZone::Right,
                            ..
                        }
                        | DropTarget::DockEdge { .. }
                        | DropTarget::FloatingGroup {
                            zone: DropZone::Center,
                            ..
                        }
                        | DropTarget::FreeArea { .. }
                ),
                DropPayload::Group(_) => matches!(&request.target, DropTarget::FreeArea { .. }),
                DropPayload::FloatingGroup(_) => matches!(
                    &request.target,
                    DropTarget::Group {
                        zone: DropZone::Center | DropZone::Top | DropZone::Bottom,
                        ..
                    } | DropTarget::TabInsertion { .. }
                        | DropTarget::Column {
                            zone: DropZone::Top
                                | DropZone::Bottom
                                | DropZone::Left
                                | DropZone::Right,
                            ..
                        }
                        | DropTarget::DockEdge { .. }
                        | DropTarget::FloatingGroup {
                            zone: DropZone::Center,
                            ..
                        }
                ),
            };

            if supported_target {
                let result = match (request.payload, request.target) {
                    (DropPayload::Panel(panel), DropTarget::FreeArea { bounds }) => {
                        // An individual dock tab detaches into a bounded in-window floating group.
                        self.layout.apply(
                            DockCommand::Float {
                                source: FloatSource::Panel(panel),
                                rect: bounds,
                            },
                            &self.metrics,
                        )
                    }
                    (DropPayload::Group(group), DropTarget::FreeArea { bounds }) => {
                        // Dragging the dock group's capture zone preserves all tabs and active index.
                        self.layout.apply(
                            DockCommand::Float {
                                source: FloatSource::Group(group),
                                rect: bounds,
                            },
                            &self.metrics,
                        )
                    }
                    (DropPayload::Panel(panel), target) => {
                        self.layout.apply(DockCommand::MovePanel { panel, target }, &self.metrics)
                    }
                    (DropPayload::FloatingGroup(floating), target) => self.layout.apply(
                        DockCommand::DockFloating { floating, target },
                        &self.metrics,
                    ),
                    _ => {
                        self.floating_drag = None;
                        cx.notify();
                        return;
                    },
                };
                if let Err(error) = result {
                    eprintln!("Could not apply floating/dock drop: {error}");
                }
            }
        } else if was_dragging {
            // Releasing a floating group over non-dock space is an ordinary move.
            // Its model bounds were never changed during preview, so a rejected dock
            // command leaves its source fully intact.
            if let Some(drag) = self.floating_drag
                && let Some(source) = self.layout.floating_groups().get(drag.floating)
            {
                let rect = DockRect::new(drag.current_bounds.origin, source.bounds.size);
                if source.bounds != rect
                    && let Err(error) = self.layout.apply(
                        DockCommand::MoveFloating {
                            floating: drag.floating,
                            rect,
                        },
                        &self.metrics,
                    )
                {
                    eprintln!("Could not move floating group {:#?}: {error}", drag.floating);
                }
            }
        }

        self.floating_drag = None;
        cx.notify();
    }

    fn apply_floating_drag_preview(&self, rects: &mut LayoutRects) {
        let Some(drag) = self.floating_drag else {
            return;
        };
        if !matches!(
            self.drag_session.state(),
            DragSessionState::Dragging {
                payload: DropPayload::FloatingGroup(floating),
                ..
            } if *floating == drag.floating
        ) {
            return;
        }

        let Some(source_rect) = rects
            .floating_groups
            .iter_mut()
            .find(|rect| rect.floating == drag.floating)
        else {
            return;
        };
        let previous_bounds = source_rect.bounds;
        let next_bounds = drag.current_bounds;
        let delta_x = next_bounds.origin.x - previous_bounds.origin.x;
        let delta_y = next_bounds.origin.y - previous_bounds.origin.y;
        source_rect.bounds = next_bounds;

        for title in rects
            .floating_titles
            .iter_mut()
            .filter(|title| title.floating == drag.floating)
        {
            title.bounds.origin = next_bounds.origin;
            title.bounds.size.width = next_bounds.size.width;
        }
        for handle in rects
            .floating_resize_edges
            .iter_mut()
            .filter(|handle| handle.floating == drag.floating)
        {
            handle.bounds.origin.x += delta_x;
            handle.bounds.origin.y += delta_y;
        }
        for panel in rects
            .panels
            .iter_mut()
            .filter(|panel| panel.floating == Some(drag.floating))
        {
            panel.bounds.origin.x += delta_x;
            panel.bounds.origin.y += delta_y;
        }
    }

    fn toggle_window_menu(&mut self, cx: &mut Context<Self>) {
        self.window_menu_open = !self.window_menu_open;
        cx.notify();
    }

    fn open_or_activate_panel(&mut self, key: PanelKey, cx: &mut Context<Self>) {
        // The menu represents registered panel types, not instances. Prefer an existing
        // instance in either docked or floating layout so even non-singleton descriptors
        // do not produce accidental duplicates when selected from Window > Panels.
        let existing_panel = self
            .layout
            .panels()
            .iter()
            .find_map(|(panel_id, panel)| (panel.key() == &key).then_some(panel_id));

        let result: Result<(), String> = if let Some(panel_id) = existing_panel {
            let dock_location = self
                .layout
                .nodes()
                .iter()
                .find_map(|(group, node)| match node {
                    Node::Tabs(tabs) => tabs
                        .panels
                        .iter()
                        .position(|candidate| *candidate == panel_id)
                        .map(|index| (group, index)),
                    Node::Split(_) => None,
                });

            if let Some((group, index)) = dock_location {
                let already_active = matches!(
                    self.layout.nodes().get(group),
                    Some(Node::Tabs(tabs)) if tabs.active_index == index
                );
                if already_active {
                    Ok(())
                } else {
                    self.layout
                        .apply(DockCommand::ActivateTab { group, index }, &self.metrics)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }
            } else if let Some((floating, index)) =
                self.layout
                    .floating_groups()
                    .iter()
                    .find_map(|(floating, group)| {
                        group
                            .panels
                            .iter()
                            .position(|candidate| *candidate == panel_id)
                            .map(|index| (floating, index))
                    })
            {
                self.layout
                    .apply(
                        DockCommand::ActivateFloatingTab { floating, index },
                        &self.metrics,
                    )
                    .map(|_| ())
                    .map_err(|error| error.to_string())
                    .and_then(|()| {
                        self.layout
                            .apply(DockCommand::RaiseFloating { floating }, &self.metrics)
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                    })
            } else {
                Err(format!(
                    "panel instance {panel_id:?} is not placed in the layout"
                ))
            }
        } else {
            self.registry
                .open_command(&key, None)
                .map_err(|error| error.to_string())
                .and_then(|command| {
                    self.layout
                        .apply(command, &self.metrics)
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                })
        };

        match result {
            Ok(()) => {
                self.window_menu_open = false;
                cx.notify();
            }
            Err(error) => eprintln!("Could not open or activate panel {key:?}: {error}"),
        }
    }

    fn toggle_panel_from_menu(&mut self, key: PanelKey, cx: &mut Context<Self>) {
        let existing_panel = self
            .layout
            .panels()
            .iter()
            .find_map(|(panel_id, panel)| (panel.key() == &key).then_some(panel_id));

        if let Some(panel_id) = existing_panel {
            match self
                .layout
                .apply(DockCommand::ClosePanel { panel: panel_id }, &self.metrics)
            {
                Ok(_) => {
                    self.window_menu_open = false;
                    cx.notify();
                }
                Err(error) => eprintln!("Could not close panel {key:?} from Window menu: {error}"),
            }
        } else {
            self.open_or_activate_panel(key, cx);
        }
    }

    fn activate_floating_tab(
        &mut self,
        floating: FloatingGroupId,
        tab_index: usize,
        cx: &mut Context<Self>,
    ) {
        // A mouse-up after dragging a floating tab may synthesize a click on its
        // original tab. Do not let that stale click undo the completed transfer.
        if matches!(
            self.drag_session.state(),
            DragSessionState::Dragging {
                payload: DropPayload::Panel(_),
                ..
            } | DragSessionState::Dropped {
                payload: DropPayload::Panel(_),
                ..
            } | DragSessionState::Cancelled {
                reason: dock_core::DragCancelReason::ReleasedOutsideTarget,
            }
        ) {
            return;
        }

        match self.layout.apply(
            DockCommand::ActivateFloatingTab { floating, index: tab_index },
            &self.metrics,
        ) {
            Ok(_) => match self.layout.apply(DockCommand::RaiseFloating { floating }, &self.metrics) {
                Ok(_) => cx.notify(),
                Err(error) => eprintln!("Could not raise floating group {floating:?}: {error}"),
            },
            Err(error) => {
                eprintln!("Could not activate floating tab {tab_index} in {floating:?}: {error}")
            }
        }
    }

    fn activate_tab(&mut self, group_id: NodeId, tab_index: usize, cx: &mut Context<Self>) {
        // A mouse-up following a drag may also synthesize a click on the original tab.
        // Do not let that stale tab index undo the selection made by MovePanel.
        if matches!(
            self.drag_session.state(),
            DragSessionState::Dragging { .. } | DragSessionState::Dropped { .. }
        ) {
            return;
        }

        if let Some(Node::Tabs(tabs)) = self.layout.nodes().get(group_id) {
            if tabs.active_index == tab_index {
                return;
            }
        }

        match self.layout.apply(
            DockCommand::ActivateTab {
                group: group_id,
                index: tab_index,
            },
            &self.metrics,
        ) {
            Ok(_) => cx.notify(),
            Err(error) => {
                eprintln!("Could not activate tab {tab_index} in group {group_id:?}: {error}")
            }
        }
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors;
        let typography = self.theme.typography;
        let weak_window = cx.entity().downgrade();

        let toggle_menu_window = weak_window.clone();
        let on_toggle_window_menu = move |app: &mut App| {
            if let Err(error) = toggle_menu_window.update(app, |this, cx| {
                this.toggle_window_menu(cx);
            }) {
                eprintln!("Could not dispatch Window menu toggle: {error}");
            }
        };

        let open_panel_window = weak_window.clone();
        let on_open_panel = move |key: PanelKey, app: &mut App| {
            if let Err(error) = open_panel_window.update(app, |this, cx| {
                this.toggle_panel_from_menu(key.clone(), cx);
            }) {
                eprintln!("Could not dispatch panel open: {error}");
            }
        };

        let activate_tab_window = weak_window.clone();
        let on_activate_tab = move |group_id, tab_index, app: &mut App| {
            if let Err(error) = activate_tab_window.update(app, |this, cx| {
                this.activate_tab(group_id, tab_index, cx);
            }) {
                eprintln!("Could not dispatch tab activation: {error}");
            }
        };

        let start_drag_window = weak_window.clone();
        let on_start_drag = move |payload: DropPayload,
                                  position: GpuiPoint<Pixels>,
                                  window: &mut Window,
                                  app: &mut App| {
            if let Err(error) = start_drag_window.update(app, |this, cx| {
                this.begin_drag(payload, position, window, cx);
            }) {
                eprintln!("Could not begin drag: {error}");
            }
        };

        let activate_floating_window = weak_window.clone();
        let on_activate_floating_tab = move |floating: FloatingGroupId,
                                             tab_index: usize,
                                             app: &mut App| {
            if let Err(error) = activate_floating_window.update(app, |this, cx| {
                this.activate_floating_tab(floating, tab_index, cx);
            }) {
                eprintln!("Could not dispatch floating tab activation: {error}");
            }
        };

        let begin_floating_window = weak_window.clone();
        let on_begin_floating_interaction = move |floating: FloatingGroupId,
                                                  edge: Option<ResizeEdge>,
                                                  position: GpuiPoint<Pixels>,
                                                  window: &mut Window,
                                                  app: &mut App| {
            if let Err(error) = begin_floating_window.update(app, |this, cx| {
                this.begin_floating_interaction(floating, edge, position, window, cx);
            }) {
                eprintln!("Could not begin floating interaction: {error}");
            }
        };

        let focus_floating_window = weak_window.clone();
        let on_focus_floating = move |floating: FloatingGroupId, app: &mut App| {
            if let Err(error) = focus_floating_window.update(app, |this, cx| {
                this.focus_floating(floating, cx);
            }) {
                eprintln!("Could not focus floating group: {error}");
            }
        };

        let update_drag_window = weak_window.clone();
        let on_pointer_move = move |_: &MouseMoveEvent, window: &mut Window, app: &mut App| {
            let position = window.mouse_position();
            if let Err(error) = update_drag_window.update(app, |this, cx| {
                if this.floating_interaction.is_some() {
                    this.update_floating_interaction(position, window, cx);
                } else {
                    this.update_panel_drag(position, window, cx);
                }
            }) {
                eprintln!("Could not update pointer interaction: {error}");
            }
        };

        let finish_drag_window = weak_window.clone();
        let on_pointer_up = move |_: &MouseUpEvent, window: &mut Window, app: &mut App| {
            let position = window.mouse_position();
            if let Err(error) = finish_drag_window.update(app, |this, cx| {
                if this.floating_interaction.is_some() {
                    this.update_floating_interaction(position, window, cx);
                    this.floating_interaction = None;
                    cx.notify();
                } else {
                    this.finish_panel_drag(position, window, cx);
                }
            }) {
                eprintln!("Could not finish pointer interaction: {error}");
            }
        };

        let cancel_drag_window = weak_window.clone();
        let on_key_down = move |event: &KeyDownEvent, _window: &mut Window, app: &mut App| {
            if event.keystroke.key == "escape" {
                if let Err(error) = cancel_drag_window.update(app, |this, cx| {
                    this.cancel_panel_drag(cx);
                }) {
                    eprintln!("Could not cancel drag: {error}");
                }
            }
        };

        let geometry = self.window_geometry(window);
        let menu_height = geometry.menu_height;
        let panel_menu_height = geometry.panel_menu_height;
        let options_height = geometry.options_height;
        let body_height = geometry.body_height;
        let docking_viewport_width = geometry.docking_viewport_width;
        let rects = geometry.rects;

        let left_dock_width = dock_side_width(&rects, DockSide::Left);
        let right_dock_width = dock_side_width(&rects, DockSide::Right);
        let canvas_width = rects.canvas.size.width.max(0.0);

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(colors.window_background))
            .text_color(rgb(colors.primary_text))
            .text_size(px(typography.body_size))
            .font_family(typography.family)
            .child(menu_bar(&self.theme, menu_height, on_toggle_window_menu))
            .when(self.window_menu_open, |element| {
                element.child(panels_menu(
                    &self.theme,
                    &self.registry,
                    &self.layout,
                    panel_menu_height,
                    on_open_panel,
                ))
            })
            .child(options_bar(&self.theme, options_height))
            .child(
                div()
                    .w_full()
                    .h(px(body_height))
                    .flex()
                    .flex_row()
                    .child(toolbar(&self.theme, body_height))
                    .child(
                        div()
                            .w(px(docking_viewport_width))
                            .h(px(body_height))
                            .relative()
                            .flex()
                            .flex_row()
                            .child(render_dock_side(
                                &self.theme,
                                &self.layout,
                                &self.registry,
                                &rects,
                                DockSide::Left,
                                left_dock_width,
                                body_height,
                                on_activate_tab.clone(),
                                on_start_drag.clone(),
                            ))
                            .child(canvas(&self.theme, canvas_width, body_height))
                            .child(render_dock_side(
                                &self.theme,
                                &self.layout,
                                &self.registry,
                                &rects,
                                DockSide::Right,
                                right_dock_width,
                                body_height,
                                on_activate_tab,
                                on_start_drag.clone(),
                            ))
                            .children(render_splitter_handles(&self.theme, &rects))
                            .children(render_floating_groups(
                                &self.theme,
                                &self.layout,
                                &self.registry,
                                &rects,
                                on_activate_floating_tab,
                                on_begin_floating_interaction,
                                on_focus_floating,
                                on_start_drag.clone(),
                            ))
                            .child(drop_overlay(
                                &self.theme,
                                &drop_target_rects_for_session(&rects, self.drag_session.state()),
                                self.drag_session.preview_target(),
                            )),
                    ),
            )
            .track_focus(&self.focus_handle)
            .on_key_down(on_key_down)
            .on_mouse_move(on_pointer_move)
            .on_mouse_up(MouseButton::Left, on_pointer_up)
    }
}

fn cancel_drag_interaction(
    drag_session: &mut DragSession,
    floating_drag: &mut Option<FloatingDragState>,
) -> bool {
    if !drag_session.cancel(DragCancelReason::Escape) {
        return false;
    }

    *floating_drag = None;
    true
}

/// Build transparent splitter hit-zones and their Theme-sized visual handles.
/// Group splitters already occupy their 4 px layout gap; the column separator is painted
/// here because column children otherwise touch without a visible boundary.
fn render_splitter_handles(theme: &Theme, rects: &LayoutRects) -> Vec<AnyElement> {
    let mut elements = Vec::with_capacity(rects.splitters.len() * 2);
    let colors = theme.colors;
    let thickness = theme.metrics.splitter_thickness;

    for splitter in &rects.splitters {
        let bounds = splitter.bounds;
        if !bounds.origin.x.is_finite()
            || !bounds.origin.y.is_finite()
            || !bounds.size.width.is_finite()
            || !bounds.size.height.is_finite()
            || bounds.size.width <= 0.0
            || bounds.size.height <= 0.0
        {
            continue;
        }

        let handle = div()
            .id(format!("dock-splitter-hit-{:?}-{}", splitter.kind, splitter.index))
            .absolute()
            .left(px(bounds.origin.x))
            .top(px(bounds.origin.y))
            .w(px(bounds.size.width))
            .h(px(bounds.size.height));

        let handle = match splitter.kind {
            SplitterKind::Group => handle.cursor_ns_resize(),
            SplitterKind::Column => handle.cursor_col_resize(),
        };
        elements.push(handle.into_any_element());

        if let Some(visible) = splitter_visible_bounds(splitter, thickness) {
            elements.push(
                div()
                    .id(format!(
                        "dock-splitter-visible-{:?}-{}",
                        splitter.kind, splitter.index
                    ))
                    .absolute()
                    .left(px(visible.origin.x))
                    .top(px(visible.origin.y))
                    .w(px(visible.size.width))
                    .h(px(visible.size.height))
                    .cursor_col_resize()
                    .bg(rgb(colors.splitter))
                    .into_any_element(),
            );
        }
    }

    elements
}

/// Return the visible vertical line centered inside a column split's wider hit-zone.
fn splitter_visible_bounds(splitter: &SplitterRect, requested_thickness: f32) -> Option<DockRect> {
    if splitter.kind != SplitterKind::Column
        || !requested_thickness.is_finite()
        || requested_thickness <= 0.0
    {
        return None;
    }

    let bounds = splitter.bounds;
    if !bounds.origin.x.is_finite()
        || !bounds.origin.y.is_finite()
        || !bounds.size.width.is_finite()
        || !bounds.size.height.is_finite()
        || bounds.size.width <= 0.0
        || bounds.size.height <= 0.0
    {
        return None;
    }

    let width = requested_thickness.min(bounds.size.width);
    Some(DockRect::new(
        DockPoint::new(
            bounds.origin.x + (bounds.size.width - width) * 0.5,
            bounds.origin.y,
        ),
        DockSize::new(width, bounds.size.height),
    ))
}

fn titlebar_resize_edge(
    point: DockPoint,
    bounds: DockRect,
    requested_hit_zone: f32,
) -> Option<ResizeEdge> {
    let hit = if requested_hit_zone.is_finite() {
        requested_hit_zone.max(0.0)
    } else {
        0.0
    };
    let width = bounds.size.width.max(0.0);
    let height = bounds.size.height.max(0.0);
    if hit <= 0.0 || width <= 0.0 || height <= 0.0 {
        return None;
    }

    let top = bounds.origin.y;
    let top_hit_bottom = top + hit.min(height);
    if point.y < top || point.y > top_hit_bottom {
        return None;
    }

    let corner_width = hit.min(width * 0.5);
    let left = bounds.origin.x;
    let right = left + width;
    if point.x <= left + corner_width {
        Some(ResizeEdge::TopLeft)
    } else if point.x >= right - corner_width {
        Some(ResizeEdge::TopRight)
    } else {
        Some(ResizeEdge::Top)
    }
}

fn drop_target_rects_for_session(
    rects: &LayoutRects,
    state: &DragSessionState,
) -> LayoutRects {
    let payload = match state {
        DragSessionState::Pressed { payload, .. }
        | DragSessionState::Dragging { payload, .. } => *payload,
        _ => return rects.clone(),
    };
    drop_target_rects(rects, payload)
}

fn drop_target_rects(rects: &LayoutRects, payload: DropPayload) -> LayoutRects {
    let mut target_rects = rects.clone();
    let source_floating = match payload {
        DropPayload::FloatingGroup(floating) => Some(floating),
        DropPayload::Panel(panel) => rects
            .panels
            .iter()
            .find(|rect| rect.panel == panel)
            .and_then(|rect| rect.floating),
        DropPayload::Group(_) => None,
    };
    if let Some(source) = source_floating {
        // A dragged floating group or one of its tabs is no longer a drop occluder.
        // This lets a tab be returned to a dock group even when that dock lies directly
        // beneath the floating window, while the window remains visible as a drag source.
        target_rects
            .floating_groups
            .retain(|rect| rect.floating != source);
        target_rects
            .floating_titles
            .retain(|title| title.floating != source);
        target_rects
            .floating_resize_edges
            .retain(|handle| handle.floating != source);
        target_rects
            .panels
            .retain(|panel| panel.floating != Some(source));
    }
    target_rects
}

fn floating_drag_preview_bounds(
    initial_bounds: DockRect,
    pointer_origin: DockPoint,
    point: DockPoint,
    viewport: DockSize,
) -> DockRect {
    let viewport_width = viewport.width.max(0.0);
    let viewport_height = viewport.height.max(0.0);
    let width = initial_bounds.size.width.max(0.0).min(viewport_width);
    let height = initial_bounds.size.height.max(0.0).min(viewport_height);
    let max_x = (viewport_width - width).max(0.0);
    let max_y = (viewport_height - height).max(0.0);
    let x = (initial_bounds.origin.x + point.x - pointer_origin.x).clamp(0.0, max_x);
    let y = (initial_bounds.origin.y + point.y - pointer_origin.y).clamp(0.0, max_y);
    DockRect::new(DockPoint::new(x, y), DockSize::new(width, height))
}

fn resize_floating_rect(
    initial: DockRect,
    edge: ResizeEdge,
    delta: DockPoint,
    minimum: DockSize,
    viewport: DockSize,
) -> DockRect {
    let viewport_width = viewport.width.max(0.0);
    let viewport_height = viewport.height.max(0.0);
    let min_width = minimum.width.max(0.0).min(viewport_width);
    let min_height = minimum.height.max(0.0).min(viewport_height);

    let mut left = initial.origin.x.clamp(0.0, viewport_width);
    let mut top = initial.origin.y.clamp(0.0, viewport_height);
    let mut right = (initial.origin.x + initial.size.width).clamp(left, viewport_width);
    let mut bottom = (initial.origin.y + initial.size.height).clamp(top, viewport_height);

    match edge {
        ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft => {
            left = (left + delta.x).clamp(0.0, (right - min_width).max(0.0));
        }
        _ => {}
    }
    match edge {
        ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight => {
            right = (right + delta.x)
                .clamp((left + min_width).min(viewport_width), viewport_width);
        }
        _ => {}
    }
    match edge {
        ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight => {
            top = (top + delta.y).clamp(0.0, (bottom - min_height).max(0.0));
        }
        _ => {}
    }
    match edge {
        ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight => {
            bottom = (bottom + delta.y)
                .clamp((top + min_height).min(viewport_height), viewport_height);
        }
        _ => {}
    }

    DockRect::new(
        DockPoint::new(left, top),
        DockSize::new((right - left).max(0.0), (bottom - top).max(0.0)),
    )
}

/// Match pure tab-header rectangles to the tab-strip's actual GPUI flex row.
/// Core geometry intentionally has no dependency on UI spacing or the group-menu button;
/// this adapter applies those view metrics before hit-testing and painting insertion markers.
fn align_tab_header_rects(
    layout: &Layout,
    rects: &mut LayoutRects,
    content_padding: f32,
    action_width: f32,
) {
    const BORDER_WIDTH: f32 = 1.0;

    for strip in &rects.tab_strips {
        let Some(Node::Tabs(tabs)) = layout.nodes().get(strip.group) else {
            continue;
        };
        let tab_count = tabs.panels.len();
        if tab_count == 0 {
            continue;
        }

        let inset = content_padding.max(0.0) + BORDER_WIDTH;
        let available_width =
            (strip.bounds.size.width - inset * 2.0 - action_width.max(0.0)).max(0.0);
        // Each tab's flex item adds horizontal padding, while the trailing spacer does not.
        // With flex-basis 0, that padding makes each rendered tab wider than the spacer:
        // tab_width = (remaining_width + both tab paddings) / (tab_count + 1).
        // Include it here so hit targets and insertion markers follow the rendered edges.
        let tab_padding = content_padding.max(0.0);
        let slot_width =
            (available_width + tab_padding * 2.0) / (tab_count + 1) as f32;

        for header in rects
            .tab_headers
            .iter_mut()
            .filter(|header| header.group == strip.group)
        {
            let left = strip.bounds.origin.x + inset + slot_width * header.index as f32;
            header.bounds = DockRect::new(
                DockPoint::new(left, strip.bounds.origin.y),
                DockSize::new(slot_width, strip.bounds.size.height),
            );
        }
    }
}

fn menu_bar<OnWindowClick>(
    theme: &Theme,
    height: f32,
    on_window_click: OnWindowClick,
) -> impl IntoElement
where
    OnWindowClick: Fn(&mut App) + Clone + 'static,
{
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;
    let labels = [
        "File", "Edit", "Image", "Layer", "Type", "Select", "Filter", "View", "Window", "Help",
    ];

    div()
        .w_full()
        .h(px(height))
        .flex()
        .items_center()
        .px(px(metrics.content_padding))
        .bg(rgb(colors.toolbar_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.body_size))
        .children(labels.into_iter().map(|label| {
            let click = on_window_click.clone();
            let is_window_menu = label == "Window";
            div()
                .id(format!("menu-item-{label}"))
                .h(px(height))
                .px(px(metrics.button_gap + metrics.content_padding * 0.5))
                .items_center()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(colors.hover)))
                .on_click(move |_, _, app| {
                    if is_window_menu {
                        click(app);
                    }
                })
                .child(label)
        }))
}

fn panels_menu<OnOpen>(
    theme: &Theme,
    registry: &PanelRegistry,
    layout: &Layout,
    height: f32,
    on_open: OnOpen,
) -> impl IntoElement
where
    OnOpen: Fn(PanelKey, &mut App) + Clone + 'static,
{
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;
    let row_height = metrics.tab_strip_height.max(24.0);

    let rows = registry.descriptors().into_iter().map(|descriptor| {
        let key = descriptor.key().clone();
        let title = descriptor.title().to_owned();
        let is_open = panel_is_open(layout, &key);

        let open_key = key.clone();
        let open = on_open.clone();

        div()
            .id(format!("window-panel-{}", key.as_str()))
            .w_full()
            .h(px(row_height))
            .flex()
            .flex_row()
            .items_center()
            .px(px(metrics.content_padding))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(colors.hover)))
            .on_click(move |_, _, app| open(open_key.clone(), app))
            .child(
                div()
                    .w(px(20.0))
                    .h(px(row_height))
                    .flex()
                    .items_center()
                    .text_color(rgb(colors.primary_text))
                    .child(if is_open { "✓" } else { "" }),
            )
            .child(
                div()
                    .flex_1()
                    .h(px(row_height))
                    .flex()
                    .items_center()
                    .child(title),
            )
    });

    div()
        .w(px(280.0))
        .h(px(height))
        .flex()
        .flex_col()
        .overflow_hidden()
        .py(px(metrics.content_padding * 0.5))
        .bg(rgb(colors.group_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.body_size))
        .text_color(rgb(colors.primary_text))
        .child(
            div()
                .h(px(row_height))
                .px(px(metrics.content_padding))
                .flex()
                .items_center()
                .text_size(px(typography.tab_size))
                .text_color(rgb(colors.muted_text))
                .child("Panels"),
        )
        .children(rows)
}

fn panel_is_open(layout: &Layout, key: &PanelKey) -> bool {
    layout.panels().values().any(|panel| panel.key() == key)
}

fn options_bar(theme: &Theme, height: f32) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    div()
        .w_full()
        .h(px(height))
        .flex()
        .items_center()
        .px(px(metrics.content_padding))
        .bg(rgb(colors.dock_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.tab_size))
        .text_color(rgb(colors.muted_text))
        .child("Options")
}

fn toolbar(theme: &Theme, height: f32) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;

    // Empty fixed rail: Toolbar is app chrome, not a dock node or a floating panel.
    div()
        .w(px(metrics.rail_width))
        .h(px(height))
        .bg(rgb(colors.toolbar_background))
        .border_1()
        .border_color(rgb(colors.border))
}

fn canvas(theme: &Theme, width: f32, height: f32) -> impl IntoElement {
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    div()
        .w(px(width))
        .h(px(height))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(colors.canvas_background))
        .border_1()
        .border_color(rgb(colors.border))
        .text_size(px(typography.title_size))
        .child(
            div()
                .px(px(metrics.content_padding))
                .text_color(rgb(colors.muted_text))
                .child("Canvas"),
        )
}

fn dock_side_width(rects: &LayoutRects, side: DockSide) -> f32 {
    rects
        .dock_areas
        .iter()
        .filter(|area| area.side == side)
        .map(|area| area.bounds.size.width.max(0.0))
        .sum()
}

fn render_dock_side<OnActivate, OnStartDrag>(
    theme: &Theme,
    layout: &Layout,
    registry: &PanelRegistry,
    rects: &LayoutRects,
    side: DockSide,
    width: f32,
    height: f32,
    on_activate: OnActivate,
    on_start_drag: OnStartDrag,
) -> impl IntoElement
where
    OnActivate: Fn(NodeId, usize, &mut App) + Clone + 'static,
    OnStartDrag: Fn(DropPayload, GpuiPoint<Pixels>, &mut Window, &mut App) + Clone + 'static,
{
    let areas = layout
        .dock_areas()
        .iter()
        .filter(|area| area.side == side)
        .map(|area| {
            let area_width = rects
                .dock_areas
                .iter()
                .find(|bounds| bounds.side == side)
                .map(|bounds| bounds.bounds.size.width.max(0.0))
                .unwrap_or_else(|| {
                    area.columns
                        .iter()
                        .map(|column| column.width.max(0.0))
                        .sum()
                });

            let columns = area
                .columns
                .iter()
                .enumerate()
                .map(|(column_index, column)| {
                    let column_bounds = rects
                        .columns
                        .iter()
                        .find(|bounds| bounds.side == side && bounds.index == column_index);
                    let column_width = column_bounds
                        .map(|bounds| bounds.bounds.size.width.max(0.0))
                        .unwrap_or_else(|| column.width.max(0.0));
                    let column_height = column_bounds
                        .map(|bounds| bounds.bounds.size.height.max(0.0))
                        .unwrap_or(height);
                    let header_height = rects
                        .column_headers
                        .iter()
                        .find(|bounds| bounds.side == side && bounds.index == column_index)
                        .map(|bounds| bounds.bounds.size.height.max(0.0))
                        .unwrap_or(theme.metrics.column_header_height);

                    let groups = column
                        .groups
                        .iter()
                        .copied()
                        .map(|group_id| {
                            let group_height = rects
                                .groups
                                .iter()
                                .find(|bounds| bounds.group == group_id)
                                .map(|bounds| bounds.bounds.size.height.max(0.0))
                                .unwrap_or(0.0);
                            let strip_height = rects
                                .tab_strips
                                .iter()
                                .find(|strip| strip.group == group_id)
                                .map(|strip| strip.bounds.size.height.max(0.0))
                                .unwrap_or(theme.metrics.tab_strip_height)
                                .min(group_height);

                            group_view_data(layout, registry, group_id, group_height, strip_height)
                        })
                        .collect::<Vec<_>>();

                    dock_column_view(
                        theme,
                        column_width,
                        column_height,
                        header_height,
                        groups,
                        on_activate.clone(),
                        on_start_drag.clone(),
                    )
                });

            div()
                .w(px(area_width))
                .h(px(height))
                .flex()
                .flex_row()
                .children(columns)
        });

    div()
        .w(px(width))
        .h(px(height))
        .flex()
        .flex_row()
        .bg(rgb(theme.colors.dock_background))
        .children(areas)
}

fn render_floating_groups<
    OnActivateFloating,
    OnBeginFloating,
    OnFocusFloating,
    OnStartDrag,
>(
    theme: &Theme,
    layout: &Layout,
    registry: &PanelRegistry,
    rects: &LayoutRects,
    on_activate: OnActivateFloating,
    on_begin_interaction: OnBeginFloating,
    on_focus_floating: OnFocusFloating,
    on_start_drag: OnStartDrag,
) -> Vec<AnyElement>
where
    OnActivateFloating: Fn(FloatingGroupId, usize, &mut App) + Clone + 'static,
    OnBeginFloating: Fn(
            FloatingGroupId,
            Option<ResizeEdge>,
            GpuiPoint<Pixels>,
            &mut Window,
            &mut App,
        ) + Clone
        + 'static,
    OnFocusFloating: Fn(FloatingGroupId, &mut App) + Clone + 'static,
    OnStartDrag: Fn(DropPayload, GpuiPoint<Pixels>, &mut Window, &mut App) + Clone + 'static,
{
    let colors = theme.colors;
    let metrics = theme.metrics;
    let typography = theme.typography;

    let mut floating_groups: Vec<_> = layout
        .floating_groups()
        .iter()
        .filter_map(|(floating_id, floating)| {
            let bounds = rects
                .floating_groups
                .iter()
                .find(|rect| rect.floating == floating_id)
                .map(|rect| rect.bounds)?;
            let title_height = rects
                .floating_titles
                .iter()
                .find(|title| title.floating == floating_id)
                .map(|title| title.bounds.size.height)
                .filter(|height| height.is_finite() && *height > 0.0)
                .unwrap_or(metrics.column_header_height)
                .min(bounds.size.height.max(0.0));

            let tabs: Vec<_> = floating
                .panels
                .iter()
                .copied()
                .enumerate()
                .filter_map(|(index, panel_id)| {
                    let panel = layout.panels().get(panel_id)?;
                    let title = registry
                        .get(panel.key())
                        .map(|descriptor| descriptor.title().to_owned())
                        .unwrap_or_else(|| panel.key().as_str().to_owned());
                    Some((index, panel_id, title, index == floating.active_index))
                })
                .collect();

            let active_panel = floating
                .panels
                .get(floating.active_index)
                .and_then(|panel_id| layout.panels().get(*panel_id));
            let fallback_heading = active_panel
                .map(|panel| {
                    registry
                        .get(panel.key())
                        .map(|descriptor| descriptor.title().to_owned())
                        .unwrap_or_else(|| panel.key().as_str().to_owned())
                })
                .unwrap_or_else(|| "Panel".to_owned());
            let (heading, message) = active_panel
                .and_then(|panel| {
                    registry
                        .create_content(panel.key())
                        .ok()
                        .and_then(|content| {
                            content
                                .as_ref()
                                .as_any()
                                .downcast_ref::<PlaceholderPanelContent>()
                                .map(|placeholder| {
                                    (placeholder.heading.clone(), placeholder.message.clone())
                                })
                        })
                })
                .unwrap_or((fallback_heading, "Panel placeholder".to_owned()));

            Some((
                floating.z_order,
                floating_id,
                bounds,
                title_height,
                tabs,
                heading,
                message,
            ))
        })
        .collect();
    // Later elements paint above earlier ones, matching the model's z-order contract.
    floating_groups.sort_by_key(|(z_order, ..)| *z_order);

    floating_groups
        .into_iter()
        .map(move |(_, floating_id, bounds, title_height, tabs, heading, message)| {
                let width = bounds.size.width.max(0.0);
                let height = bounds.size.height.max(0.0);
                let tab_height = metrics
                    .tab_strip_height
                    .max(0.0)
                    .min((height - title_height).max(0.0));
                let content_height = (height - title_height - tab_height).max(0.0);

                let tab_elements = tabs.into_iter().map(
                    |(index, panel_id, title, active)| {
                        let activate = on_activate.clone();
                        let start_drag = on_start_drag.clone();
                        div()
                            .id(format!("floating-tab-{floating_id:?}-{index}"))
                            .flex_1()
                            .min_w(px(0.0))
                            .h(px(tab_height))
                            .flex()
                            .items_center()
                            .px(px(metrics.content_padding))
                            .bg(rgb(if active {
                                colors.active_tab
                            } else {
                                colors.group_background
                            }))
                            .text_color(rgb(if active {
                                colors.primary_text
                            } else {
                                colors.muted_text
                            }))
                            .text_size(px(typography.tab_size))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(colors.hover)))
                            .on_mouse_down(MouseButton::Left, move |_, window, app| {
                                start_drag(
                                    DropPayload::Panel(panel_id),
                                    window.mouse_position(),
                                    window,
                                    app,
                                )
                            })
                            .on_click(move |_, _, app| {
                                activate(floating_id, index, app)
                            })
                            .child(title)
                    },
                );

                div()
                    .id(format!("floating-group-{floating_id:?}"))
                    .absolute()
                    .left(px(bounds.origin.x))
                    .top(px(bounds.origin.y))
                    .w(px(width))
                    .h(px(height))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .bg(rgb(colors.group_background))
                    .border_1()
                    .border_color(rgb(colors.border))
                    .child({
                        let begin_move = on_begin_interaction.clone();
                        div()
                            .w_full()
                            .h(px(title_height))
                            .flex()
                            .items_center()
                            .px(px(metrics.content_padding))
                            .bg(rgb(colors.dock_background))
                            .border_1()
                            .border_color(rgb(colors.border))
                            .text_size(px(typography.tab_size))
                            .text_color(rgb(colors.muted_text))
                            .cursor_move()
                            .hover(|style| style.bg(rgb(colors.hover)))
                            .on_mouse_down(MouseButton::Left, move |_, window, app| {
                                begin_move(
                                    floating_id,
                                    None,
                                    window.mouse_position(),
                                    window,
                                    app,
                                )
                            })
                            .child(format!("Floating · {heading}"))
                    })
                    .child(
                        div()
                            .w_full()
                            .h(px(tab_height))
                            .flex()
                            .flex_row()
                            .items_center()
                            .overflow_hidden()
                            .bg(rgb(colors.group_background))
                            .border_1()
                            .border_color(rgb(colors.border))
                            .children(tab_elements),
                    )
                    .child({
                        let focus = on_focus_floating.clone();
                        div()
                            .w_full()
                            .h(px(content_height))
                            .flex()
                            .flex_col()
                            .overflow_hidden()
                            .px(px(metrics.content_padding))
                            .py(px(metrics.content_padding))
                            .bg(rgb(colors.group_background))
                            .text_color(rgb(colors.primary_text))
                            .on_mouse_down(MouseButton::Left, move |_, _, app| {
                                focus(floating_id, app)
                            })
                            .child(
                                div()
                                    .w_full()
                                    .text_size(px(typography.title_size))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(heading),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .text_size(px(typography.body_size))
                                    .text_color(rgb(colors.muted_text))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(message),
                            )
                    })
                    .children(
                        rects
                            .floating_resize_edges
                            .iter()
                            .filter(|handle| handle.floating == floating_id)
                            .map(|handle| {
                                let edge = handle.edge;
                                let start_resize = on_begin_interaction.clone();
                                let left = handle.bounds.origin.x - bounds.origin.x;
                                let top = handle.bounds.origin.y - bounds.origin.y;
                                let handle_element = div()
                                    .id(format!("floating-resize-{floating_id:?}-{edge:?}"))
                                    .absolute()
                                    .left(px(left))
                                    .top(px(top))
                                    .w(px(handle.bounds.size.width.max(0.0)))
                                    .h(px(handle.bounds.size.height.max(0.0)));

                                // Resize hit regions stay visually transparent. Only the cursor
                                // indicates the direction in which this edge/corner can move.
                                let handle_element = match edge {
                                    ResizeEdge::Left | ResizeEdge::Right => {
                                        handle_element.cursor_ew_resize()
                                    }
                                    ResizeEdge::Top | ResizeEdge::Bottom => {
                                        handle_element.cursor_ns_resize()
                                    }
                                    ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                        handle_element.cursor_nwse_resize()
                                    }
                                    ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                        handle_element.cursor_nesw_resize()
                                    }
                                };

                                handle_element
                                    .on_mouse_down(MouseButton::Left, move |_, window, app| {
                                        start_resize(
                                            floating_id,
                                            Some(edge),
                                            window.mouse_position(),
                                            window,
                                            app,
                                        )
                                    })
                                    .into_any_element()
                            }),
                    )
                    .into_any_element()
        })
        .collect()
}

fn group_view_data(
    layout: &Layout,
    registry: &PanelRegistry,
    group_id: dock_core::NodeId,
    group_height: f32,
    tab_strip_height: f32,
) -> DockGroupData {
    let mut tabs = Vec::new();
    let mut active_content = None;

    if let Some(Node::Tabs(group)) = layout.nodes().get(group_id) {
        for (index, panel_id) in group.panels.iter().copied().enumerate() {
            let Some(panel) = layout.panels().get(panel_id) else {
                continue;
            };

            let title = registry
                .get(panel.key())
                .map(|descriptor| descriptor.title().to_owned())
                .unwrap_or_else(|| panel.key().as_str().to_owned());

            if index == group.active_index {
                let fallback = (title.clone(), "Panel placeholder".to_owned());
                active_content = registry
                    .create_content(panel.key())
                    .ok()
                    .and_then(|content| {
                        content
                            .as_ref()
                            .as_any()
                            .downcast_ref::<PlaceholderPanelContent>()
                            .map(|placeholder| {
                                (placeholder.heading.clone(), placeholder.message.clone())
                            })
                    })
                    .or(Some(fallback));
            }

            tabs.push(TabItem {
                panel_id,
                title,
                active: index == group.active_index,
            });
        }
    }

    let (active_heading, active_message) =
        active_content.unwrap_or_else(|| ("Panel".to_owned(), "No panel content".to_owned()));

    DockGroupData {
        group_id,
        height: group_height,
        tab_strip_height,
        tabs,
        active_heading,
        active_message,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        align_tab_header_rects, cancel_drag_interaction, group_view_data, resize_floating_rect,
        splitter_visible_bounds, titlebar_resize_edge,
    };
    use crate::{dock_metrics, panels::PanelRegistry};
    use ui::Theme;

    #[test]
    fn escape_cancels_drag_preview_and_discards_floating_preview() {
        use dock_core::{
            DragCancelReason, DragSession, DragSessionState, DockSide, DropPayload, DropTarget,
            FloatingGroupId, Point, Rect, Size,
        };
        use slotmap::Key;

        let floating = FloatingGroupId::null();
        let mut session = DragSession::new();
        assert!(session.press(DropPayload::FloatingGroup(floating), Point::new(10.0, 10.0)));
        assert!(session.pointer_moved(
            Point::new(20.0, 10.0),
            Some(DropTarget::DockEdge { side: DockSide::Left }),
        ));
        let mut preview = Some(super::FloatingDragState {
            floating,
            pointer_origin: Point::new(10.0, 10.0),
            initial_bounds: Rect::new(Point::new(20.0, 30.0), Size::new(320.0, 240.0)),
            current_bounds: Rect::new(Point::new(30.0, 30.0), Size::new(320.0, 240.0)),
        });

        assert!(cancel_drag_interaction(&mut session, &mut preview));
        assert_eq!(
            session.state(),
            &DragSessionState::Cancelled {
                reason: DragCancelReason::Escape
            }
        );
        assert!(session.preview_target().is_none());
        assert!(preview.is_none());
        assert!(session.release().is_none());
        assert!(!cancel_drag_interaction(&mut session, &mut preview));
    }

    #[test]
    fn column_splitter_draws_a_four_pixel_line_centered_in_the_eight_pixel_hit_zone() {
        use dock_core::{Point, Rect, Size, SplitterKind, SplitterRect};

        let splitter = SplitterRect {
            kind: SplitterKind::Column,
            index: 1,
            bounds: Rect::new(Point::new(96.0, 0.0), Size::new(8.0, 640.0)),
        };
        let visible = splitter_visible_bounds(&splitter, 4.0).expect("column line is visible");

        assert_eq!(
            visible,
            Rect::new(Point::new(98.0, 0.0), Size::new(4.0, 640.0)),
            "the visible 4 px line is centered in the 8 px capture zone"
        );
        assert!(
            splitter_visible_bounds(&splitter, f32::NAN).is_none(),
            "invalid theme thickness must not produce invalid drawing geometry"
        );
        assert!(
            splitter_visible_bounds(
                &SplitterRect { kind: SplitterKind::Group, ..splitter },
                4.0,
            )
            .is_none(),
            "this helper draws column lines only; group separators already render in the column"
        );
    }

    #[test]
    fn titlebar_top_hit_band_routes_to_resize_edges_not_move() {
        use dock_core::{Point, Rect, ResizeEdge, Size};

        let bounds = Rect::new(Point::new(100.0, 80.0), Size::new(300.0, 200.0));
        assert_eq!(
            titlebar_resize_edge(Point::new(103.0, 84.0), bounds, 8.0),
            Some(ResizeEdge::TopLeft)
        );
        assert_eq!(
            titlebar_resize_edge(Point::new(250.0, 84.0), bounds, 8.0),
            Some(ResizeEdge::Top)
        );
        assert_eq!(
            titlebar_resize_edge(Point::new(397.0, 84.0), bounds, 8.0),
            Some(ResizeEdge::TopRight)
        );
        assert_eq!(titlebar_resize_edge(Point::new(250.0, 100.0), bounds, 8.0), None);
        assert_eq!(titlebar_resize_edge(Point::new(250.0, 79.0), bounds, 8.0), None);
        assert_eq!(titlebar_resize_edge(Point::new(250.0, 84.0), bounds, 0.0), None);
    }

    #[test]
    fn floating_resize_anchors_opposite_edge_and_clamps_to_viewport() {
        use dock_core::{Point, Rect, ResizeEdge, Size};

        let initial = Rect::new(Point::new(100.0, 80.0), Size::new(300.0, 200.0));
        let from_left = resize_floating_rect(
            initial,
            ResizeEdge::Left,
            Point::new(50.0, 0.0),
            Size::new(180.0, 130.0),
            Size::new(500.0, 400.0),
        );
        assert_eq!(from_left, Rect::new(Point::new(150.0, 80.0), Size::new(250.0, 200.0)));

        let from_top = resize_floating_rect(
            initial,
            ResizeEdge::Top,
            Point::new(0.0, 500.0),
            Size::new(180.0, 130.0),
            Size::new(500.0, 400.0),
        );
        assert_eq!(from_top, Rect::new(Point::new(100.0, 150.0), Size::new(300.0, 130.0)));

        let from_right = resize_floating_rect(
            initial,
            ResizeEdge::Right,
            Point::new(1_000.0, 0.0),
            Size::new(180.0, 130.0),
            Size::new(500.0, 400.0),
        );
        assert_eq!(from_right, Rect::new(Point::new(100.0, 80.0), Size::new(400.0, 200.0)));
    }

    #[test]
    fn tab_header_drop_geometry_matches_rendered_tab_strip_flex_layout() {
        let theme = Theme::default();
        let metrics = dock_metrics::build(&theme);
        let registry = PanelRegistry::with_builtins().expect("built-in descriptors are valid");
        let layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("initial dock layout is valid");
        let group = layout.dock_areas()[0].columns[0].groups[0];
        let mut rects =
            dock_core::compute_rects(&layout, dock_core::Size::new(1000.0, 600.0), &metrics);
        align_tab_header_rects(
            &layout,
            &mut rects,
            theme.metrics.content_padding,
            theme.metrics.button_size,
        );

        let strip = rects
            .tab_strips
            .iter()
            .find(|strip| strip.group == group)
            .expect("first group has a tab strip");
        let mut headers: Vec<_> = rects
            .tab_headers
            .iter()
            .filter(|header| header.group == group)
            .collect();
        headers.sort_by_key(|header| header.index);
        let inset = theme.metrics.content_padding + 1.0;
        let available_width =
            strip.bounds.size.width - inset * 2.0 - theme.metrics.button_size;
        // The two rendered tabs carry horizontal padding; the trailing flex spacer does not.
        let expected_slot_width =
            (available_width + theme.metrics.content_padding * 2.0) / 3.0;

        assert!((headers[0].bounds.origin.x - (strip.bounds.origin.x + inset)).abs() < 0.001);
        assert!((headers[0].bounds.size.width - expected_slot_width).abs() < 0.001);
        assert!(
            (headers[1].bounds.origin.x
                - (headers[0].bounds.origin.x + headers[0].bounds.size.width))
                .abs()
                < 0.001
        );
    }

    #[test]
    fn active_panel_projection_tracks_each_groups_independent_layout_state() {
        use dock_core::{DockCommand, Node};

        let theme = Theme::default();
        let metrics = dock_metrics::build(&theme);
        let registry = PanelRegistry::with_builtins().expect("built-in descriptors are valid");
        let mut layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("initial dock layout is valid");
        let groups = layout.dock_areas()[0].columns[0].groups.clone();

        assert_eq!(groups.len(), 2);

        let upper_initial = group_view_data(
            &layout,
            &registry,
            groups[0],
            220.0,
            theme.metrics.tab_strip_height,
        );
        assert_eq!(
            upper_initial
                .tabs
                .iter()
                .map(|tab| tab.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Layers", "Color"]
        );
        assert!(upper_initial.tabs[0].active);
        assert!(!upper_initial.tabs[1].active);
        assert_eq!(upper_initial.active_heading, "Layers");
        assert_eq!(
            upper_initial.active_message,
            "Layer stack and visibility controls."
        );

        let lower_initial = group_view_data(
            &layout,
            &registry,
            groups[1],
            220.0,
            theme.metrics.tab_strip_height,
        );
        assert_eq!(
            lower_initial
                .tabs
                .iter()
                .map(|tab| tab.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Properties", "History"]
        );
        assert!(lower_initial.tabs[0].active);
        assert_eq!(lower_initial.active_heading, "Properties");

        let original_version = layout.version();
        layout
            .apply(
                DockCommand::ActivateTab {
                    group: groups[0],
                    index: 1,
                },
                &metrics,
            )
            .expect("the Color tab can be activated");
        assert!(layout.version() > original_version);

        let upper_after_activation = group_view_data(
            &layout,
            &registry,
            groups[0],
            220.0,
            theme.metrics.tab_strip_height,
        );
        let lower_after_activation = group_view_data(
            &layout,
            &registry,
            groups[1],
            220.0,
            theme.metrics.tab_strip_height,
        );

        assert!(!upper_after_activation.tabs[0].active);
        assert!(upper_after_activation.tabs[1].active);
        assert_eq!(upper_after_activation.active_heading, "Color");
        assert_eq!(
            upper_after_activation.active_message,
            "Color selection and foreground/background swatches."
        );
        assert!(lower_after_activation.tabs[0].active);
        assert!(!lower_after_activation.tabs[1].active);
        assert_eq!(lower_after_activation.active_heading, "Properties");

        layout
            .apply(
                DockCommand::ActivateTab {
                    group: groups[1],
                    index: 1,
                },
                &metrics,
            )
            .expect("the History tab can be activated");

        let upper_after_second_activation = group_view_data(
            &layout,
            &registry,
            groups[0],
            220.0,
            theme.metrics.tab_strip_height,
        );
        let lower_after_second_activation = group_view_data(
            &layout,
            &registry,
            groups[1],
            220.0,
            theme.metrics.tab_strip_height,
        );

        assert!(upper_after_second_activation.tabs[1].active);
        assert_eq!(upper_after_second_activation.active_heading, "Color");
        assert!(!lower_after_second_activation.tabs[0].active);
        assert!(lower_after_second_activation.tabs[1].active);
        assert_eq!(lower_after_second_activation.active_heading, "History");
        assert_eq!(
            lower_after_second_activation.active_message,
            "Recent editing operations."
        );
    }
    #[test]
    fn panel_menu_commands_close_reopen_and_activate_without_duplicates() {
        use dock_core::{DockCommand, Node, PanelKey};

        let theme = Theme::default();
        let metrics = dock_metrics::build(&theme);
        let registry = PanelRegistry::with_builtins().expect("built-in descriptors are valid");
        let mut layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("initial dock layout is valid");
        let initial_groups = layout.dock_areas()[0].columns[0].groups.clone();
        let color_key = PanelKey::new("color").expect("valid panel key");

        let color_panel = layout
            .panels()
            .iter()
            .find_map(|(panel_id, panel)| (panel.key() == &color_key).then_some(panel_id))
            .expect("Color starts open");
        layout
            .apply(DockCommand::ClosePanel { panel: color_panel }, &metrics)
            .expect("ClosePanel should close Color");
        assert_eq!(layout.panels().len(), 3);
        assert!(
            !layout
                .panels()
                .iter()
                .any(|(_, panel)| panel.key() == &color_key)
        );

        let open_command = registry
            .open_command(&color_key, None)
            .expect("registered Color has an open command");
        layout
            .apply(open_command, &metrics)
            .expect("OpenPanel should reopen Color");
        assert_eq!(layout.panels().len(), 4);

        let color_panel = layout
            .panels()
            .iter()
            .find_map(|(panel_id, panel)| (panel.key() == &color_key).then_some(panel_id))
            .expect("Color should exist after reopening");
        let (group_id, tab_index, active_index) = layout
            .nodes()
            .iter()
            .find_map(|(group_id, node)| match node {
                Node::Tabs(tabs) => tabs
                    .panels
                    .iter()
                    .position(|panel_id| *panel_id == color_panel)
                    .map(|index| (group_id, index, tabs.active_index)),
                Node::Split(_) => None,
            })
            .expect("reopened Color should belong to a tab group");
        assert_eq!(group_id, initial_groups[0]);
        assert_eq!(tab_index, 1);
        assert_eq!(active_index, 1);

        // OpenPanel remains safe if another caller requests an already-open singleton.
        let activate_existing = registry
            .open_command(&color_key, None)
            .expect("registered Color has an open command");
        layout
            .apply(activate_existing, &metrics)
            .expect("OpenPanel should activate existing Color");
        assert_eq!(layout.panels().len(), 4);
    }

    #[test]
    fn window_panel_menu_marks_registered_open_panels_and_excludes_canvas() {
        use super::panel_is_open;
        use dock_core::{DockCommand, PanelKey};

        let theme = Theme::default();
        let metrics = crate::dock_metrics::build(&theme);
        let registry = PanelRegistry::with_builtins().expect("built-in descriptors are valid");
        let mut layout = registry
            .build_initial_photoshop_layout(&metrics)
            .expect("initial dock layout is valid");

        assert!(
            registry
                .descriptors()
                .iter()
                .all(|descriptor| descriptor.key().as_str() != "canvas"),
            "Canvas is window chrome and must not appear in the registered-panel menu"
        );

        let layers_key = PanelKey::new("layers").expect("built-in key is valid");
        let color_key = PanelKey::new("color").expect("built-in key is valid");
        assert!(panel_is_open(&layout, &layers_key));
        assert!(panel_is_open(&layout, &color_key));

        let color_panel = layout
            .panels()
            .iter()
            .find_map(|(panel_id, panel)| (panel.key() == &color_key).then_some(panel_id))
            .expect("Color panel is open");
        layout
            .apply(DockCommand::ClosePanel { panel: color_panel }, &metrics)
            .expect("an ordinary panel can be closed");

        assert!(panel_is_open(&layout, &layers_key));
        assert!(!panel_is_open(&layout, &color_key));
    }
}
