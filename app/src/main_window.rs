use std::collections::{HashMap, HashSet};

use dock_core::{
    create_initial_layout, Axis, DockCommand, DockMetrics, DockSide, DragDropAction,
    DragDropRequest, DragHandle, DragPayload, DragSession, DragState, DropSource, DropTarget,
    FloatingGroupId, FlyoutState, Layout, LayoutRects, Node, NodeId, PanelId, PanelKey, Point,
    Profile, Rect, ResizeEdge, Size,
};
use gpui::{
    App, ClickEvent, Context, FocusHandle, InteractiveElement, IntoElement, MouseDownEvent,
    ParentElement, Render, ScrollHandle, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px, rgb,
};
use ui::Theme;

use crate::panels::{PanelMenuItem, PanelRegistry};

const MENU_BAR_HEIGHT: f32 = 28.0;
const MENU_WIDTH: f32 = 232.0;
const MENU_ROW_HEIGHT: f32 = 28.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DockMenuState {
    WindowPanels,
    TabOverflow { group: NodeId },
    PanelGroup { group: NodeId, panel: Option<PanelId> },
}

impl From<dock_gpui::DockMenuRequest> for DockMenuState {
    fn from(request: dock_gpui::DockMenuRequest) -> Self {
        match request {
            dock_gpui::DockMenuRequest::TabOverflow { group } => Self::TabOverflow { group },
            dock_gpui::DockMenuRequest::PanelGroup { group, panel } => {
                Self::PanelGroup { group, panel }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EscapeAction {
    CancelInteraction,
    DismissMenu,
    DismissFlyout,
    Ignore,
}

fn escape_action(interaction_active: bool, menu_open: bool, flyout_open: bool) -> EscapeAction {
    if interaction_active {
        EscapeAction::CancelInteraction
    } else if menu_open {
        EscapeAction::DismissMenu
    } else if flyout_open {
        EscapeAction::DismissFlyout
    } else {
        EscapeAction::Ignore
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FocusDirection {
    Forward,
    Backward,
}

fn focus_direction(shift_pressed: bool) -> FocusDirection {
    if shift_pressed {
        FocusDirection::Backward
    } else {
        FocusDirection::Forward
    }
}

#[derive(Clone)]
enum MenuAction {
    OpenOrActivatePanel(PanelKey),
    Command(DockCommand),
}

struct MenuRow {
    id: String,
    label: String,
    action: MenuAction,
}

#[derive(Clone, Copy, Debug)]
enum ResizeOperation {
    GroupBoundary {
        column: NodeId,
        boundary_index: usize,
        available_height: f32,
    },
    ColumnBoundary {
        side: DockSide,
        boundary_index: usize,
    },
    OuterColumn {
        column: NodeId,
        side: DockSide,
        initial_width: f32,
    },
    FloatingBounds {
        group: FloatingGroupId,
        edge: ResizeEdge,
        initial_bounds: Rect,
    },
}

#[derive(Clone, Copy, Debug)]
struct ResizeGesture {
    operation: ResizeOperation,
    origin: Point,
    latest: Point,
    last_applied: Point,
}


fn scroll_content_position(
    position: Point,
    viewport: Rect,
    scroll_offset_y: f32,
) -> Option<Point> {
    if !position.is_finite()
        || !scroll_offset_y.is_finite()
        || !viewport.contains(position).unwrap_or(false)
    {
        return None;
    }

    let content_y = position.y - scroll_offset_y;
    content_y
        .is_finite()
        .then_some(Point::new(position.x, content_y))
}

/// The app is the only owner of the live Layout and applies adapter-returned commands.
pub(crate) struct MainWindow {
    layout: Layout,
    theme: Theme,
    metrics: DockMetrics,
    panels: PanelRegistry,
    scroll_handles: HashMap<NodeId, ScrollHandle>,
    menu: Option<DockMenuState>,
    drag_session: DragSession,
    resize_gesture: Option<ResizeGesture>,
    resize_frame_scheduled: bool,
    focus_handle: Option<FocusHandle>,
    _focus_subscription: Option<Subscription>,
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let panels = PanelRegistry::default();
        let metrics = dock_gpui::dock_metrics_from_theme(&theme);
        let descriptors = panels.core_descriptors();
        let layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
            .map_err(str::to_owned)?;
        let drag_session = DragSession::new(&metrics)
            .map_err(|error| format!("cannot create drag session: {error:?}"))?;

        Ok(Self {
            layout,
            theme,
            metrics,
            panels,
            scroll_handles: HashMap::new(),
            menu: None,
            drag_session,
            resize_gesture: None,
            resize_frame_scheduled: false,
            focus_handle: None,
            _focus_subscription: None,
        })
    }

    fn core_drop_position(&self, position: Point, rects: &LayoutRects) -> Point {
        if !position.is_finite()
            || rects
                .floating_groups
                .iter()
                .any(|floating| floating.rect.contains(position).unwrap_or(false))
        {
            return position;
        }

        for overflow in &rects.vertical_overflow {
            if !matches!(self.layout.node(overflow.owner), Some(Node::Column(_))) {
                continue;
            }
            let Some(scroll_handle) = self.scroll_handles.get(&overflow.owner) else {
                continue;
            };
            if let Some(content_position) = scroll_content_position(
                position,
                overflow.viewport,
                scroll_handle.offset().y.as_f32(),
            ) {
                return content_position;
            }
        }
        position
    }

    fn group_scroll_offset(&self, group: NodeId, rects: &LayoutRects) -> f32 {
        for overflow in &rects.vertical_overflow {
            if !matches!(self.layout.node(overflow.owner), Some(Node::Column(_)))
                || !self.is_main_area_column(overflow.owner)
            {
                continue;
            }
            let Some(scroll_handle) = self.scroll_handles.get(&overflow.owner) else {
                continue;
            };
            if self
                .layout
                .column_display_order(overflow.owner)
                .is_ok_and(|items| items.iter().any(|item| item.group == group))
            {
                return scroll_handle.offset().y.as_f32();
            }
        }
        0.0
    }

    fn is_main_area_column(&self, column_id: NodeId) -> bool {
        [DockSide::Left, DockSide::Right].into_iter().any(|side| {
            let Some(area) = self.layout.main_area(side) else {
                return false;
            };
            match self.layout.node(area.root) {
                Some(Node::Column(_)) => area.root == column_id,
                Some(Node::Split(split))
                    if split.axis == Axis::X
                        && split
                            .children
                            .iter()
                            .all(|child| matches!(self.layout.node(*child), Some(Node::Column(_)))) =>
                {
                    split.children.contains(&column_id)
                }
                _ => false,
            }
        })
    }

    fn apply_command(&mut self, command: DockCommand, cx: &mut Context<Self>) {
        self.menu = None;
        match self.layout.apply(command, &self.metrics) {
            Ok(_) => cx.notify(),
            Err(error) => {
                eprintln!("Dock command rejected by Core: {error:?}");
                cx.notify();
            }
        }
    }

    fn begin_drag(
        &mut self,
        payload: DragPayload,
        handle: DragHandle,
        position: Point,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.resize_gesture.is_some() {
            return;
        }
        match self.drag_session.state() {
            DragState::Pressed { .. } | DragState::Dragging { .. } => return,
            DragState::Dropped { .. } | DragState::Cancelled { .. } => {
                if self.drag_session.reset().is_err() {
                    return;
                }
            }
            DragState::Idle => {}
        }
        if let Err(error) = self
            .drag_session
            .pointer_down_with_handle(payload, handle, position)
        {
            eprintln!("Could not begin dock drag: {error:?}");
            return;
        }
        if let Some(focus_handle) = self.focus_handle.as_ref() {
            window.focus(focus_handle, cx);
        }
    }

    fn update_drag_position(
        &mut self,
        position: Point,
        viewport: Rect,
        cx: &mut Context<Self>,
    ) {
        let Some(payload) = active_drag_payload(self.drag_session.state()) else {
            return;
        };
        let previous_state = self.drag_session.state();
        if let Err(error) = self.drag_session.pointer_move(position) {
            eprintln!("Could not update dock drag: {error:?}");
            return;
        }

        if matches!(self.drag_session.state(), DragState::Dragging { .. })
            && !matches!(payload, DragPayload::FloatingGroup { .. })
        {
            let target = source_for_payload(payload).and_then(|source| {
                let rects = self.layout.compute_rects(viewport, &self.metrics).ok()?;
                let core_position = self.core_drop_position(position, &rects);
                self.layout
                    .resolve_drop(core_position, source, &rects, &self.metrics)
                    .ok()
                    .flatten()
            });
            let _ = self.drag_session.set_drop_target(target);
        }
        if matches!(previous_state, DragState::Dragging { .. })
            || matches!(self.drag_session.state(), DragState::Dragging { .. })
        {
            cx.notify();
        }
    }

    fn finish_drag_at(
        &mut self,
        position: Point,
        viewport: Rect,
        cx: &mut Context<Self>,
    ) {
        let Some(payload) = active_drag_payload(self.drag_session.state()) else {
            return;
        };

        let geometry = self.layout.compute_rects(viewport, &self.metrics).ok();
        let target = if matches!(payload, DragPayload::FloatingGroup { .. }) {
            None
        } else {
            source_for_payload(payload).and_then(|source| {
                let rects = geometry.as_ref()?;
                let core_position = self.core_drop_position(position, rects);
                self.layout
                    .resolve_drop(core_position, source, rects, &self.metrics)
                    .ok()
                    .flatten()
            })
        };

        let mut command = None;
        match self.drag_session.pointer_up(position, target) {
            Ok(Some(mut request)) => {
                if request.target == Some(DropTarget::FreeArea) {
                    let floating_bounds = geometry.as_ref().and_then(|rects| {
                        self.floating_bounds_for_request(&request, viewport, rects)
                    });
                    request.floating_bounds = floating_bounds;
                }
                match self.layout.resolve_drag_drop(&request, &self.metrics) {
                    Ok(DragDropAction::NoOp) => {}
                    Ok(DragDropAction::Command(resolved)) => command = Some(resolved),
                    Err(error) => eprintln!("Dock drop rejected by Core: {error:?}"),
                }
            }
            Ok(None) => {}
            Err(error) => eprintln!("Could not finish dock drag: {error:?}"),
        }

        let _ = self.drag_session.reset();
        cx.notify();
        if let Some(command) = command {
            self.apply_command(command, cx);
        }
    }

    fn floating_bounds_for_request(
        &self,
        request: &DragDropRequest,
        viewport: Rect,
        rects: &LayoutRects,
    ) -> Option<Rect> {
        let size = match request.payload {
            DragPayload::Panel { panel } => {
                let instance = self.layout.panel_instance(panel)?;
                instance
                    .state
                    .last_known_size()
                    .unwrap_or(instance.constraints.preferred_size)
            }
            DragPayload::Group { group } => rects
                .groups
                .iter()
                .find(|entry| entry.group == group)
                .map(|entry| entry.rect.size)
                .unwrap_or(Size::new(
                    self.metrics.default_column_width,
                    self.metrics.min_group_height,
                )),
            DragPayload::FloatingGroup { group } => self.layout.floating_group(group)?.bounds.size,
        };
        if !size.is_finite() || size.width <= 0.0 || size.height <= 0.0 {
            return None;
        }
        let origin = Point::new(
            (request.point.x - size.width / 2.0)
                .clamp(0.0, (viewport.size.width - size.width).max(0.0)),
            (request.point.y - size.height / 2.0)
                .clamp(0.0, (viewport.size.height - size.height).max(0.0)),
        );
        Some(Rect::new(origin, size))
    }

    fn begin_resize(
        &mut self,
        handle: dock_gpui::ResizeHandle,
        position: Point,
        viewport: Rect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if active_drag_payload(self.drag_session.state()).is_some() || self.resize_gesture.is_some() {
            return;
        }
        if !position.is_finite() {
            return;
        }
        let Ok(rects) = self.layout.compute_rects(viewport, &self.metrics) else {
            return;
        };

        let operation = match handle {
            dock_gpui::ResizeHandle::Splitter {
                owner,
                boundary_index,
                axis: Axis::Y,
            } => {
                let Some(Node::Column(column)) = self.layout.node(owner) else {
                    return;
                };
                let Some(column_rect) = rects.columns.iter().find(|item| item.column == owner) else {
                    return;
                };
                let gaps =
                    self.metrics.splitter_thickness * column.groups.len().saturating_sub(1) as f32;
                let content_height = rects
                    .vertical_overflow
                    .iter()
                    .find(|overflow| overflow.owner == owner)
                    .map_or(column_rect.rect.size.height, |overflow| {
                        overflow.content_extent.size.height
                    });
                let available_height = content_height - gaps;
                if !available_height.is_finite() || available_height <= 0.0 {
                    return;
                }
                ResizeOperation::GroupBoundary {
                    column: owner,
                    boundary_index,
                    available_height,
                }
            }
            dock_gpui::ResizeHandle::Splitter {
                owner,
                boundary_index,
                axis: Axis::X,
            } => {
                let side = [DockSide::Left, DockSide::Right]
                    .into_iter()
                    .find(|side| self.layout.main_area(*side).is_some_and(|area| area.root == owner));
                let Some(side) = side else {
                    return;
                };
                let Some(Node::Split(split)) = self.layout.node(owner) else {
                    return;
                };
                if split.axis != Axis::X || boundary_index >= split.children.len().saturating_sub(1) {
                    return;
                }
                ResizeOperation::ColumnBoundary { side, boundary_index }
            }
            dock_gpui::ResizeHandle::OuterColumn { column, side } => {
                let Some(Node::Column(value)) = self.layout.node(column) else {
                    return;
                };
                if value.mode.is_rail() || !value.width.is_finite() || value.width <= 0.0 {
                    return;
                }
                ResizeOperation::OuterColumn {
                    column,
                    side,
                    initial_width: value.width,
                }
            }
            dock_gpui::ResizeHandle::FloatingEdge { group, edge } => {
                let Some(bounds) = rects
                    .floating_groups
                    .iter()
                    .find(|item| item.group == group)
                    .map(|item| item.rect)
                else {
                    return;
                };
                ResizeOperation::FloatingBounds {
                    group,
                    edge,
                    initial_bounds: bounds,
                }
            }
        };

        self.resize_gesture = Some(ResizeGesture {
            operation,
            origin: position,
            latest: position,
            last_applied: position,
        });
        if let Some(focus_handle) = self.focus_handle.as_ref() {
            window.focus(focus_handle, cx);
        }
    }

    fn update_resize_position(
        &mut self,
        position: Point,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut gesture) = self.resize_gesture else {
            return;
        };
        if !position.is_finite() {
            return;
        }
        gesture.latest = position;
        self.resize_gesture = Some(gesture);

        if !self.resize_frame_scheduled {
            self.resize_frame_scheduled = true;
            cx.on_next_frame(window, |view, _window, cx| {
                view.flush_resize(cx);
            });
        }
    }

    fn flush_resize(&mut self, cx: &mut Context<Self>) {
        self.resize_frame_scheduled = false;
        let Some(mut gesture) = self.resize_gesture else {
            return;
        };
        if gesture.latest == gesture.last_applied {
            return;
        }
        let Some(command) = self.resize_command(gesture) else {
            gesture.last_applied = gesture.latest;
            self.resize_gesture = Some(gesture);
            return;
        };

        match self.layout.apply(command, &self.metrics) {
            Ok(_) => {
                gesture.last_applied = gesture.latest;
                self.resize_gesture = Some(gesture);
                cx.notify();
            }
            Err(error) => {
                eprintln!("Dock resize command rejected by Core: {error:?}");
            }
        }
    }

    fn resize_command(&self, gesture: ResizeGesture) -> Option<DockCommand> {
        let dx_total = gesture.latest.x - gesture.origin.x;
        let dy_total = gesture.latest.y - gesture.origin.y;
        let dx_frame = gesture.latest.x - gesture.last_applied.x;
        let dy_frame = gesture.latest.y - gesture.last_applied.y;
        match gesture.operation {
            ResizeOperation::GroupBoundary {
                column,
                boundary_index,
                available_height,
            } => (dy_frame != 0.0).then_some(DockCommand::ResizeGroups {
                column,
                boundary_index,
                delta: dy_frame,
                available_height,
            }),
            ResizeOperation::ColumnBoundary {
                side,
                boundary_index,
            } => (dx_frame != 0.0).then_some(DockCommand::ResizeColumnBoundary {
                side,
                boundary_index,
                delta: dx_frame,
            }),
            ResizeOperation::OuterColumn {
                column,
                side,
                initial_width,
            } => {
                let directed_delta = if side == DockSide::Left { dx_total } else { -dx_total };
                (directed_delta != 0.0).then_some(DockCommand::ResizeColumn {
                    column,
                    width: initial_width + directed_delta,
                })
            }
            ResizeOperation::FloatingBounds {
                group,
                edge,
                initial_bounds,
            } => {
                let mut bounds = initial_bounds;
                let initial_right = initial_bounds.origin.x + initial_bounds.size.width;
                let initial_bottom = initial_bounds.origin.y + initial_bounds.size.height;
                match edge {
                    ResizeEdge::Left => {
                        bounds.origin.x = initial_bounds.origin.x + dx_total;
                        bounds.size.width = initial_bounds.size.width - dx_total;
                    }
                    ResizeEdge::Right => bounds.size.width = initial_bounds.size.width + dx_total,
                    ResizeEdge::Top => {
                        bounds.origin.y = initial_bounds.origin.y + dy_total;
                        bounds.size.height = initial_bounds.size.height - dy_total;
                    }
                    ResizeEdge::Bottom => bounds.size.height = initial_bounds.size.height + dy_total,
                    ResizeEdge::TopLeft => {
                        bounds.origin.x = initial_bounds.origin.x + dx_total;
                        bounds.size.width = initial_bounds.size.width - dx_total;
                        bounds.origin.y = initial_bounds.origin.y + dy_total;
                        bounds.size.height = initial_bounds.size.height - dy_total;
                    }
                    ResizeEdge::TopRight => {
                        bounds.size.width = initial_bounds.size.width + dx_total;
                        bounds.origin.y = initial_bounds.origin.y + dy_total;
                        bounds.size.height = initial_bounds.size.height - dy_total;
                    }
                    ResizeEdge::BottomLeft => {
                        bounds.origin.x = initial_bounds.origin.x + dx_total;
                        bounds.size.width = initial_bounds.size.width - dx_total;
                        bounds.size.height = initial_bounds.size.height + dy_total;
                    }
                    ResizeEdge::BottomRight => {
                        bounds.size.width = initial_bounds.size.width + dx_total;
                        bounds.size.height = initial_bounds.size.height + dy_total;
                    }
                }
                // Keep the opposite edge fixed while preventing invalid dimensions. Core
                // additionally applies panel-specific minimum-size constraints.
                let min_width = self.metrics.min_column_width.min(initial_bounds.size.width);
                let min_height = self.metrics.min_group_height.min(initial_bounds.size.height);
                if bounds.size.width < min_width {
                    bounds.size.width = min_width;
                    if matches!(edge, ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft) {
                        bounds.origin.x = initial_right - min_width;
                    }
                }
                if bounds.size.height < min_height {
                    bounds.size.height = min_height;
                    if matches!(edge, ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight) {
                        bounds.origin.y = initial_bottom - min_height;
                    }
                }
                (bounds != initial_bounds).then_some(DockCommand::ResizeFloatingBounds { group, bounds })
            }
        }
    }

    fn finish_pointer_at(&mut self, position: Point, viewport: Rect, cx: &mut Context<Self>) {
        if let Some(mut gesture) = self.resize_gesture {
            gesture.latest = position;
            self.resize_gesture = Some(gesture);
            // Flush the final position now; the already-scheduled frame becomes a no-op
            // after this gesture is cleared.
            self.flush_resize(cx);
            self.resize_gesture = None;
            self.resize_frame_scheduled = false;
            cx.notify();
            return;
        }
        self.finish_drag_at(position, viewport, cx);
    }

    fn handle_escape(&mut self, cx: &mut Context<Self>) {
        let interaction_active = active_drag_payload(self.drag_session.state()).is_some()
            || self.resize_gesture.is_some();
        match escape_action(
            interaction_active,
            self.menu.is_some(),
            self.layout.flyout().is_some(),
        ) {
            EscapeAction::CancelInteraction => self.cancel_active_interactions(true, cx),
            EscapeAction::DismissMenu => {
                self.menu = None;
                cx.notify();
            }
            EscapeAction::DismissFlyout => self.apply_command(DockCommand::DismissFlyout, cx),
            EscapeAction::Ignore => {}
        }
    }

    fn cancel_active_interactions(&mut self, escape: bool, cx: &mut Context<Self>) {
        let mut changed = false;
        if active_drag_payload(self.drag_session.state()).is_some() {
            let result = if escape {
                self.drag_session.escape()
            } else {
                self.drag_session.focus_lost()
            };
            if result.is_ok() {
                let _ = self.drag_session.reset();
                changed = true;
            }
        }
        if self.resize_gesture.take().is_some() {
            changed = true;
        }
        self.resize_frame_scheduled = false;
        if changed {
            cx.notify();
        }
    }

    fn show_menu(&mut self, menu: DockMenuState, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(menu) { None } else { Some(menu) };
        cx.notify();
    }

    fn handle_menu_action(&mut self, action: MenuAction, cx: &mut Context<Self>) {
        self.menu = None;
        cx.notify();
        match action {
            MenuAction::Command(command) => self.apply_command(command, cx),
            MenuAction::OpenOrActivatePanel(key) => self.open_or_activate_panel(key, cx),
        }
    }

    /// Window/Panels is intentionally an open-or-activate operation, never a toggle-close.
    fn open_or_activate_panel(&mut self, key: PanelKey, cx: &mut Context<Self>) {
        let existing_panel = self
            .layout
            .panel_instances()
            .find(|(_, instance)| instance.key.as_str() == key.as_str())
            .map(|(panel, _)| panel);

        let Some(panel) = existing_panel else {
            let Some(constraints) = self.panels.constraints_for_key(&key) else {
                eprintln!("No panel descriptor registered for key {}", key.as_str());
                return;
            };
            self.apply_command(
                DockCommand::OpenPanel {
                    key,
                    constraints,
                    target: None,
                },
                cx,
            );
            return;
        };

        // Rail entries are represented by transient flyouts. Re-selecting the already
        // visible item must not call OpenFlyout, because Core can auto-collapse that state.
        let mut rail_location = None;
        for (column_id, node) in self.layout.nodes() {
            let Node::Column(column) = node else {
                continue;
            };
            if !column.mode.is_rail() {
                continue;
            }
            let Ok(items) = self.layout.column_display_order(column_id) else {
                continue;
            };
            if let Some(item) = items.into_iter().find(|item| item.panel == panel) {
                rail_location = Some((column_id, item.group));
                break;
            }
        }

        if let Some((column, group)) = rail_location {
            let requested_flyout = FlyoutState::new(column, group, panel);
            if self.layout.flyout() != Some(requested_flyout) {
                self.apply_command(DockCommand::OpenFlyout { column, panel }, cx);
            } else {
                cx.notify();
            }
            return;
        }

        let panel_node = self.layout.nodes().find_map(|(node_id, node)| match node {
            Node::Panel(value) if value.panel_id == panel => Some(node_id),
            _ => None,
        });
        let group = panel_node.and_then(|panel_node| {
            self.layout.nodes().find_map(|(node_id, node)| match node {
                Node::Tabs(tabs) if tabs.children.contains(&panel_node) => Some(node_id),
                _ => None,
            })
        });

        if let Some(group) = group {
            let group_is_collapsed = matches!(
                self.layout.node(group),
                Some(Node::Tabs(tabs)) if tabs.collapsed
            );
            if group_is_collapsed {
                self.apply_command(
                    DockCommand::CollapseGroup {
                        group,
                        collapsed: false,
                    },
                    cx,
                );
            }
            self.apply_command(DockCommand::ActivateTab { group, panel }, cx);
        } else {
            // A valid open panel should always have a Tabs parent. Keep the menu responsive
            // if a future profile introduces a different tree shape.
            cx.notify();
        }
    }

    fn menu_rows(&self, rects: &LayoutRects) -> Vec<MenuRow> {
        let Some(menu) = self.menu else {
            return Vec::new();
        };

        match menu {
            DockMenuState::WindowPanels => self
                .panels
                .menu_descriptors()
                .into_iter()
                .map(|(key, title)| MenuRow {
                    id: format!("window-panel-{}", key.as_str()),
                    label: title,
                    action: MenuAction::OpenOrActivatePanel(key),
                })
                .collect(),
            DockMenuState::TabOverflow { group } => rects
                .tab_overflows
                .iter()
                .find(|overflow| overflow.group == group)
                .map(|overflow| {
                    overflow
                        .hidden_tabs
                        .iter()
                        .filter_map(|panel| {
                            self.layout.panel_instance(*panel).map(|instance| MenuRow {
                                id: format!("overflow-{:?}-{:?}", group, panel),
                                label: self.panels.title(instance),
                                action: MenuAction::Command(DockCommand::ActivateTab {
                                    group,
                                    panel: *panel,
                                }),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            DockMenuState::PanelGroup { group, panel } => {
                let mut rows = Vec::new();
                if let Some(panel) = panel
                    && let Some(instance) = self.layout.panel_instance(panel)
                {
                    rows.push(MenuRow {
                        id: format!("close-panel-{:?}", panel),
                        label: format!("Close Panel — {}", self.panels.title(instance)),
                        action: MenuAction::Command(DockCommand::ClosePanel { panel }),
                    });
                    rows.extend(self.panels.menu_items(instance).into_iter().map(
                        |PanelMenuItem { label, command }| MenuRow {
                            id: format!("panel-action-{:?}-{}", panel, label),
                            label: label.to_owned(),
                            action: MenuAction::Command(command),
                        },
                    ));
                }
                if matches!(self.layout.node(group), Some(Node::Tabs(_))) {
                    rows.push(MenuRow {
                        id: format!("close-group-{:?}", group),
                        label: "Close Tab Group".to_owned(),
                        action: MenuAction::Command(DockCommand::CloseGroup { group }),
                    });
                }
                rows
            }
        }
    }

    fn menu_anchor(&self, rects: &LayoutRects) -> (f32, f32) {
        let Some(menu) = self.menu else {
            return (0.0, MENU_BAR_HEIGHT);
        };
        match menu {
            DockMenuState::WindowPanels => (0.0, MENU_BAR_HEIGHT),
            DockMenuState::TabOverflow { group } => rects
                .tab_overflows
                .iter()
                .find(|overflow| overflow.group == group)
                .map(|overflow| {
                    (
                        overflow.button_rect.origin.x,
                        MENU_BAR_HEIGHT
                            + overflow.button_rect.origin.y
                            + overflow.button_rect.size.height
                            + self.group_scroll_offset(group, rects),
                    )
                })
                .unwrap_or((0.0, MENU_BAR_HEIGHT)),
            DockMenuState::PanelGroup { group, .. } => {
                if let Some(flyout) = rects.flyout
                    && flyout.group == group
                {
                    return (
                        flyout.rect.origin.x + flyout.rect.size.width - MENU_WIDTH,
                        MENU_BAR_HEIGHT
                            + flyout.rect.origin.y
                            + self.theme.metrics.column_header_height,
                    );
                }
                rects
                    .groups
                    .iter()
                    .find(|candidate| candidate.group == group)
                    .map(|candidate| {
                        (
                            candidate.rect.origin.x + candidate.rect.size.width - MENU_WIDTH,
                            MENU_BAR_HEIGHT
                                + candidate.rect.origin.y
                                + self.theme.metrics.column_header_height
                                + self.group_scroll_offset(group, rects),
                        )
                    })
                    .unwrap_or((0.0, MENU_BAR_HEIGHT))
            }
        }
    }
}

fn active_drag_payload(state: DragState) -> Option<DragPayload> {
    match state {
        DragState::Pressed { payload, .. } | DragState::Dragging { payload, .. } => Some(payload),
        DragState::Idle | DragState::Dropped { .. } | DragState::Cancelled { .. } => None,
    }
}

fn source_for_payload(payload: DragPayload) -> Option<DropSource> {
    match payload {
        DragPayload::Panel { panel } => Some(DropSource::Panel { panel }),
        DragPayload::Group { group } => Some(DropSource::Group { group }),
        DragPayload::FloatingGroup { .. } => None,
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_size = window.viewport_size();
        let width = window_size.width.as_f32().max(1.0);
        let height = window_size.height.as_f32().max(1.0);
        let dock_height = (height - MENU_BAR_HEIGHT).max(1.0);
        let dock_viewport = Rect::new(Point::ZERO, Size::new(width, dock_height));

        let rects = match self.layout.compute_rects(dock_viewport, &self.metrics) {
            Ok(rects) => rects,
            Err(error) => {
                return div()
                    .w(px(width))
                    .h(px(height))
                    .bg(rgb(self.theme.colors.window_background))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(format!("Dock geometry error: {error:?}"))
                    .into_any_element();
            }
        };

        let live_columns: HashSet<NodeId> = self
            .layout
            .nodes()
            .filter_map(|(id, node)| matches!(node, Node::Column(_)).then_some(id))
            .collect();
        self.scroll_handles
            .retain(|column, _| live_columns.contains(column));
        for column in &rects.columns {
            self.scroll_handles
                .entry(column.column)
                .or_insert_with(ScrollHandle::new);
        }

        if self.focus_handle.is_none() {
            let focus_handle = cx.focus_handle();
            let subscription = cx.on_focus_out(
                &focus_handle,
                window,
                |view, _event, _window, cx| {
                    view.cancel_active_interactions(false, cx);
                },
            );
            self.focus_handle = Some(focus_handle);
            self._focus_subscription = Some(subscription);
        }
        let root_focus_handle = self
            .focus_handle
            .as_ref()
            .expect("focus handle initialized above")
            .clone();

        let drag_viewport = dock_viewport;
        let move_weak = cx.weak_entity();
        let resize_move_weak = cx.weak_entity();
        let release_weak = cx.weak_entity();
        let escape_weak = cx.weak_entity();
        let mut root = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.window_background))
            .track_focus(&root_focus_handle)
            .id("main-window-drag-root")
            .on_drag_move::<DragPayload>(move |event, _window, app| {
                let position = Point::new(
                    event.event.position.x.as_f32(),
                    event.event.position.y.as_f32() - MENU_BAR_HEIGHT,
                );
                let weak = move_weak.clone();
                let _ = weak.update(app, |view, cx| {
                    view.update_drag_position(position, drag_viewport, cx);
                });
            })
            .on_mouse_move(move |event, window, app| {
                let position = Point::new(
                    event.position.x.as_f32(),
                    event.position.y.as_f32() - MENU_BAR_HEIGHT,
                );
                let weak = resize_move_weak.clone();
                let _ = weak.update(app, |view, cx| {
                    view.update_resize_position(position, window, cx);
                });
            })
            .capture_any_mouse_up(move |event, _window, app| {
                if event.button != gpui::MouseButton::Left {
                    return;
                }
                let position = Point::new(
                    event.position.x.as_f32(),
                    event.position.y.as_f32() - MENU_BAR_HEIGHT,
                );
                let weak = release_weak.clone();
                let _ = weak.update(app, |view, cx| {
                    view.finish_pointer_at(position, drag_viewport, cx);
                });
            })
            .on_key_down(move |event, window, app| {
                match event.keystroke.key.as_str() {
                    "tab" => match focus_direction(event.keystroke.modifiers.shift) {
                        FocusDirection::Forward => window.focus_next(app),
                        FocusDirection::Backward => window.focus_prev(app),
                    },
                    "escape" => {
                        let weak = escape_weak.clone();
                        let _ = weak.update(app, |view, cx| {
                            view.handle_escape(cx);
                        });
                    }
                    _ => {}
                }
            });

        let window_menu_weak = cx.weak_entity();
        let window_menu_button = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .w(px(128.0))
            .h(px(MENU_BAR_HEIGHT))
            .bg(rgb(self.theme.colors.toolbar_background))
            .text_color(rgb(self.theme.colors.primary_text))
            .child("Window / Panels  ▾")
            .id("window-panels-menu-trigger")
            .focusable()
            .focus_visible(|style| style.border_2().border_color(rgb(self.theme.colors.focus)))
            .on_click(move |_event: &ClickEvent, _window: &mut Window, app: &mut App| {
                let weak = window_menu_weak.clone();
                let _ = weak.update(app, |view, cx| {
                    view.show_menu(DockMenuState::WindowPanels, cx);
                });
            });
        root = root.child(window_menu_button.into_any_element());

        let drag_overlay = self.drag_session.preview().and_then(|preview| {
            let source = source_for_payload(preview.payload)?;
            let zones = self.layout.compute_drop_zones(source, &rects, &self.metrics).ok()?;
            Some(dock_gpui::DragOverlay { preview, zones })
        });

        let panel_registry = &self.panels;
        let theme = &self.theme;
        let command_weak = cx.weak_entity();
        let menu_weak = cx.weak_entity();
        let drag_start_weak = cx.weak_entity();
        let resize_start_weak = cx.weak_entity();
        let dock_view = dock_gpui::render(
            &self.layout,
            &rects,
            &self.scroll_handles,
            theme,
            |instance| panel_registry.render(instance, theme),
            |instance| panel_registry.title(instance),
            |instance| panel_registry.icon(instance),
            move |command| {
                let weak = command_weak.clone();
                Box::new(move |_event: &ClickEvent, _window: &mut Window, app: &mut App| {
                    let _ = weak.update(app, |view, cx| {
                        view.apply_command(command.clone(), cx);
                    });
                }) as dock_gpui::CommandClickHandler
            },
            move |request| {
                let weak = menu_weak.clone();
                Box::new(move |_event: &ClickEvent, _window: &mut Window, app: &mut App| {
                    let _ = weak.update(app, |view, cx| {
                        view.show_menu(request.into(), cx);
                    });
                }) as dock_gpui::CommandClickHandler
            },
            move |payload, handle| {
                let weak = drag_start_weak.clone();
                Box::new(move |event: &MouseDownEvent, window: &mut Window, app: &mut App| {
                    let position = Point::new(
                        event.position.x.as_f32(),
                        event.position.y.as_f32() - MENU_BAR_HEIGHT,
                    );
                    let _ = weak.update(app, |view, cx| {
                        view.begin_drag(payload, handle, position, window, cx);
                    });
                }) as dock_gpui::DragStartHandler
            },
            move |handle| {
                let weak = resize_start_weak.clone();
                Box::new(move |event: &MouseDownEvent, window: &mut Window, app: &mut App| {
                    let position = Point::new(
                        event.position.x.as_f32(),
                        event.position.y.as_f32() - MENU_BAR_HEIGHT,
                    );
                    let _ = weak.update(app, |view, cx| {
                        view.begin_resize(handle, position, drag_viewport, window, cx);
                    });
                }) as dock_gpui::ResizeStartHandler
            },
            drag_overlay.as_ref(),
        );
        root = root.child(
            div()
                .absolute()
                .left(px(0.0))
                .top(px(MENU_BAR_HEIGHT))
                .w(px(width))
                .h(px(dock_height))
                .child(dock_view),
        );

        if let Some(menu) = self.menu {
            let rows = self.menu_rows(&rects);
            let menu_height = rows.len().max(1) as f32 * MENU_ROW_HEIGHT;
            let (anchor_x, anchor_y) = self.menu_anchor(&rects);
            let menu_x = anchor_x.clamp(0.0, (width - MENU_WIDTH).max(0.0));
            let max_y = (height - menu_height).max(MENU_BAR_HEIGHT);
            let menu_y = anchor_y.clamp(MENU_BAR_HEIGHT, max_y);
            let mut menu_element = div()
                .absolute()
                .left(px(menu_x))
                .top(px(menu_y))
                .w(px(MENU_WIDTH))
                .h(px(menu_height))
                .bg(rgb(self.theme.colors.dock_background))
                .border_1()
                .border_color(rgb(self.theme.colors.border));

            if rows.is_empty() {
                menu_element = menu_element.child(
                    div()
                        .w(px(MENU_WIDTH))
                        .h(px(MENU_ROW_HEIGHT))
                        .text_color(rgb(self.theme.colors.muted_text))
                        .child(match menu {
                            DockMenuState::TabOverflow { .. } => "No hidden tabs",
                            DockMenuState::PanelGroup { .. } => "No panel actions",
                            DockMenuState::WindowPanels => "No panels available",
                        }),
                );
            } else {
                for row in rows {
                    let action = row.action;
                    let weak = cx.weak_entity();
                    let row_element = div()
                        .w(px(MENU_WIDTH))
                        .h(px(MENU_ROW_HEIGHT))
                        .text_color(rgb(self.theme.colors.primary_text))
                        .child(row.label)
                        .id(format!("dock-menu-row-{}", row.id))
                        .focusable()
                        .focus_visible(|style| {
                            style.border_2().border_color(rgb(self.theme.colors.focus))
                        })
                        .on_click(move |_event: &ClickEvent, _window: &mut Window, app: &mut App| {
                            let weak = weak.clone();
                            let _ = weak.update(app, |view, cx| {
                                view.handle_menu_action(action.clone(), cx);
                            });
                        });
                    menu_element = menu_element.child(row_element);
                }
            }
            root = root.child(menu_element);
        }

        root.into_any_element()
    }
}

#[cfg(test)]
mod scroll_position_tests {
    use super::scroll_content_position;
    use dock_core::{Point, Rect, Size};

    #[test]
    fn screen_position_maps_back_to_scrolled_core_geometry() {
        let viewport = Rect::new(Point::new(20.0, 10.0), Size::new(120.0, 80.0));
        assert_eq!(
            scroll_content_position(Point::new(40.0, 40.0), viewport, -25.0),
            Some(Point::new(40.0, 65.0))
        );
        assert_eq!(
            scroll_content_position(Point::new(40.0, 5.0), viewport, -25.0),
            None
        );
        assert_eq!(
            scroll_content_position(Point::new(40.0, 40.0), viewport, f32::NAN),
            None
        );
    }
}

#[cfg(test)]
mod keyboard_handling_tests {
    use super::{EscapeAction, FocusDirection, escape_action, focus_direction};

    #[test]
    fn escape_cancels_active_drag_or_resize_before_dismissing_overlays() {
        assert_eq!(
            escape_action(true, true, true),
            EscapeAction::CancelInteraction,
        );
        assert_eq!(
            escape_action(true, false, true),
            EscapeAction::CancelInteraction,
        );
    }

    #[test]
    fn escape_dismisses_menu_before_flyout() {
        assert_eq!(
            escape_action(false, true, true),
            EscapeAction::DismissMenu,
        );
        assert_eq!(
            escape_action(false, false, true),
            EscapeAction::DismissFlyout,
        );
        assert_eq!(
            escape_action(false, false, false),
            EscapeAction::Ignore,
        );
    }

    #[test]
    fn shift_tab_reverses_focus_traversal() {
        assert_eq!(focus_direction(false), FocusDirection::Forward);
        assert_eq!(focus_direction(true), FocusDirection::Backward);
    }
}
