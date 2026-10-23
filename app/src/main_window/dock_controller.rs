use std::collections::{HashMap, HashSet};

use dock_core::{
    create_initial_layout, Axis, DockCommand, DockMetrics, DockSide,
    DragDropAction, DragDropRequest, DragHandle, DragPayload, DragSession, DragState, DropSource,
    DropTarget, Layout, LayoutRects, Node, PanelConstraints, PanelInstance, PanelKey, Point,
    Profile, Rect, ResizeEdge, Size, UndoRedoHistory,
};
use dock_gpui::{
    CommandClickHandler, DockMenuRequest, DragOverlay, DragStartHandler, ResizeHandle,
    ResizeStartHandler, dock_metrics_from_theme, normalize_docked_group_geometry, render,
};
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point as GpuiPoint,
    FocusHandle, ScrollHandle, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};
use ui::Theme;

use super::MainWindow;

/// Owns the app-side connection between the Core model and the GPUI adapter.
///
/// Core remains the source of truth for panel identity, tree structure, geometry and commands.
/// This controller only supplies panel presentation and translates user input into Core actions.
pub(crate) struct DockController {
    theme: Theme,
    metrics: DockMetrics,
    layout: Layout,
    history: UndoRedoHistory,
    scroll_handles: HashMap<dock_core::NodeId, ScrollHandle>,
    current_rects: Option<LayoutRects>,
    drag_session: Option<DragSession>,
    drag_overlay: Option<DragOverlay>,
    resize_gesture: Option<ResizeGesture>,
    menu_request: Option<DockMenuRequest>,
    focus_handle: Option<FocusHandle>,
}

struct ResizeGesture {
    handle: ResizeHandle,
    origin: Point,
    original_layout: Layout,
    original_rects: LayoutRects,
}

impl DockController {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let metrics = dock_metrics_from_theme(&theme);
        metrics
            .validate()
            .map_err(|error| format!("Invalid dock metrics: {error:?}"))?;

        // The descriptors are app-owned metadata. Layout/tree state is created by Core.
        let constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(metrics.default_column_width, 220.0),
            None,
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| {
                PanelKey::new(key)
                    .map(|key| (key, constraints))
                    .map_err(|_| format!("Invalid built-in panel key: {key}"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
            .map_err(str::to_owned)?;
        layout
            .validate(&metrics)
            .map_err(|errors| format!("Initial dock layout is invalid: {errors:?}"))?;

        Ok(Self {
            theme,
            metrics,
            layout,
            history: UndoRedoHistory::new(),
            scroll_handles: HashMap::new(),
            current_rects: None,
            drag_session: None,
            drag_overlay: None,
            resize_gesture: None,
            menu_request: None,
            focus_handle: None,
        })
    }

    pub(crate) fn render(
        &mut self,
        cx: &mut Context<MainWindow>,
        width: f32,
        height: f32,
    ) -> Div {
        let width = width.max(1.0);
        let height = height.max(1.0);
        if self.focus_handle.is_none() {
            self.focus_handle = Some(cx.focus_handle());
        }
        let focus_handle = self
            .focus_handle
            .as_ref()
            .expect("focus handle was initialized above")
            .clone();
        let viewport = Rect::new(Point::ZERO, Size::new(width, height));

        let mut rects = match self.layout.compute_rects(viewport, &self.metrics) {
            Ok(rects) => rects,
            Err(error) => {
                return div()
                    .w(px(width))
                    .h(px(height))
                    .bg(rgb(self.theme.colors.canvas_background))
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(format!("Could not compute dock geometry: {error:?}"));
            }
        };
        normalize_docked_group_geometry(&self.layout, &self.metrics, &mut rects);
        self.update_scroll_handles(&rects);
        self.current_rects = Some(rects.clone());

        let theme = self.theme;
        let entity = cx.entity();

        let make_command_handler = {
            let entity = entity.clone();
            move |command: DockCommand| -> CommandClickHandler {
                let entity = entity.clone();
                Box::new(move |_, _, app| {
                    let command = command.clone();
                    entity.update(app, move |window, cx| {
                        window.dock.apply_command(command);
                        cx.notify();
                    });
                })
            }
        };

        let make_menu_handler = {
            let entity = entity.clone();
            move |request: DockMenuRequest| -> CommandClickHandler {
                let entity = entity.clone();
                Box::new(move |_, _, app| {
                    entity.update(app, |window, cx| {
                        window.dock.menu_request = Some(request);
                        cx.notify();
                    });
                })
            }
        };

        let make_drag_start_handler = {
            let entity = entity.clone();
            move |payload: DragPayload, handle: DragHandle| -> DragStartHandler {
                let entity = entity.clone();
                Box::new(move |event: &MouseDownEvent, _, app| {
                    let position = core_point(event.position);
                    entity.update(app, |window, cx| {
                        window.dock.begin_drag(payload, handle, position);
                        cx.notify();
                    });
                })
            }
        };

        let make_resize_start_handler = {
            let entity = entity.clone();
            move |handle: ResizeHandle| -> ResizeStartHandler {
                let entity = entity.clone();
                Box::new(move |event: &MouseDownEvent, _, app| {
                    let position = core_point(event.position);
                    entity.update(app, |window, cx| {
                        window.dock.begin_resize(handle, position);
                        cx.notify();
                    });
                })
            }
        };

        let render_command_handler = make_command_handler.clone();
        let render_menu_handler = make_menu_handler.clone();
        let mut root = render(
            &self.layout,
            &rects,
            &self.scroll_handles,
            &theme,
            |instance| render_panel(instance, theme),
            |instance| panel_title(instance.key.as_str()).to_owned(),
            |instance| panel_icon(instance.key.as_str()),
            render_command_handler,
            render_menu_handler,
            make_drag_start_handler,
            make_resize_start_handler,
            self.drag_overlay.as_ref(),
        );

        if let Some(request) = self.menu_request {
            if let Some(menu) = self.render_menu(request, &rects, &make_command_handler, &entity) {
                root = root.child(menu);
            }
        }

        // GPUI delivers pointer events to this full-window root while the adapter owns
        // the individual hit surfaces. This joins those events to Core's drag session.
        root = root
            .track_focus(&focus_handle)
            .on_mouse_move(cx.listener(
                |window, event: &MouseMoveEvent, _, cx| {
                    if window.dock.pointer_move(core_point(event.position)) {
                        cx.notify();
                    }
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|window, event: &MouseUpEvent, _, cx| {
                    if window.dock.pointer_up(core_point(event.position)) {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|window, event: &MouseUpEvent, _, cx| {
                    if window.dock.pointer_up(core_point(event.position)) {
                        cx.notify();
                    }
                }),
            )
            .on_key_down(cx.listener(|window, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key.as_str() == "escape"
                    && window.dock.escape_or_dismiss()
                {
                    cx.notify();
                }
            }));

        root
    }

    fn update_scroll_handles(&mut self, rects: &LayoutRects) {
        let owners = rects
            .vertical_overflow
            .iter()
            .map(|overflow| overflow.owner)
            .collect::<HashSet<_>>();
        self.scroll_handles
            .retain(|owner, _| owners.contains(owner));
        for owner in owners {
            self.scroll_handles
                .entry(owner)
                .or_insert_with(ScrollHandle::new);
        }
    }

    fn apply_command(&mut self, command: DockCommand) {
        self.menu_request = None;
        if let Err(error) = self
            .history
            .apply(&mut self.layout, command, &self.metrics)
        {
            eprintln!("Dock command rejected: {error}");
        }
    }

    fn begin_drag(&mut self, payload: DragPayload, handle: DragHandle, position: Point) {
        self.resize_gesture = None;
        self.drag_overlay = None;
        let Ok(mut session) = DragSession::new(&self.metrics) else {
            return;
        };
        if let Err(error) = session.pointer_down_with_handle(payload, handle, position) {
            eprintln!("Could not start dock drag: {error}");
            return;
        }
        self.drag_session = Some(session);
    }

    fn begin_resize(&mut self, handle: ResizeHandle, position: Point) {
        let Some(rects) = self.current_rects.clone() else {
            return;
        };
        self.drag_session = None;
        self.drag_overlay = None;
        self.menu_request = None;
        self.resize_gesture = Some(ResizeGesture {
            handle,
            origin: position,
            original_layout: self.layout.clone(),
            original_rects: rects,
        });
    }

    fn pointer_move(&mut self, position: Point) -> bool {
        if self.resize_gesture.is_some() {
            self.update_resize_preview(position);
            return true;
        }
        if self.drag_session.is_some() {
            self.refresh_drag_target(position);
            return true;
        }
        false
    }

    fn refresh_drag_target(&mut self, position: Point) {
        let mut next_overlay = None;
        {
            let Some(session) = self.drag_session.as_mut() else {
                return;
            };
            if session.pointer_move(position).is_err() {
                return;
            }

            if let DragState::Dragging { payload, .. } = session.state() {
                let zones = match (payload, self.current_rects.as_ref()) {
                    (DragPayload::Panel { panel }, Some(rects)) => self
                        .layout
                        .compute_drop_zones(DropSource::Panel { panel }, rects, &self.metrics)
                        .unwrap_or_default(),
                    (DragPayload::Group { group }, Some(rects)) => self
                        .layout
                        .compute_drop_zones(DropSource::Group { group }, rects, &self.metrics)
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };

                if !matches!(payload, DragPayload::FloatingGroup { .. }) {
                    let target = zones
                        .iter()
                        .filter(|zone| rect_contains(zone.rect, position))
                        .min_by(|left, right| {
                            rect_area(left.rect)
                                .partial_cmp(&rect_area(right.rect))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .map(|zone| zone.target);
                    let _ = session.set_drop_target(target);
                }

                if let Some(preview) = session.preview() {
                    next_overlay = Some(DragOverlay { preview, zones });
                }
            }
        }
        self.drag_overlay = next_overlay;
    }

    fn pointer_up(&mut self, position: Point) -> bool {
        if self.resize_gesture.is_some() {
            self.finish_resize(position);
            return true;
        }
        if self.drag_session.is_some() {
            self.finish_drag(position);
            return true;
        }
        false
    }

    fn finish_drag(&mut self, position: Point) {
        self.refresh_drag_target(position);
        let Some(mut session) = self.drag_session.take() else {
            self.drag_overlay = None;
            return;
        };
        let target = session.preview().and_then(|preview| preview.target);
        let request = match session.pointer_up(position, target) {
            Ok(Some(request)) => request,
            Ok(None) => {
                self.drag_overlay = None;
                return;
            }
            Err(error) => {
                eprintln!("Dock drag cancelled: {error}");
                self.drag_overlay = None;
                return;
            }
        };

        self.apply_drag_request(request);
        self.drag_overlay = None;
    }

    fn apply_drag_request(&mut self, mut request: DragDropRequest) {
        if request.target == Some(DropTarget::FreeArea) {
            request.floating_bounds = Some(self.floating_bounds_at(request.point));
        }

        match self.layout.resolve_drag_drop(&request, &self.metrics) {
            Ok(DragDropAction::NoOp) => {}
            Ok(DragDropAction::Command(command)) => self.apply_command(command),
            Err(error) => eprintln!("Dock drop rejected: {error}"),
        }
    }

    fn floating_bounds_at(&self, point: Point) -> Rect {
        let viewport = self
            .current_rects
            .as_ref()
            .and_then(|rects| rects.viewport)
            .unwrap_or(Rect::new(Point::ZERO, Size::new(1200.0, 800.0)));
        let size = Size::new(420.0_f32.min(viewport.size.width), 320.0_f32.min(viewport.size.height));
        let origin = Point::new(
            (point.x - size.width / 2.0).clamp(viewport.origin.x, viewport.origin.x + (viewport.size.width - size.width).max(0.0)),
            (point.y - size.height / 2.0).clamp(viewport.origin.y, viewport.origin.y + (viewport.size.height - size.height).max(0.0)),
        );
        Rect::new(origin, size)
    }

    fn update_resize_preview(&mut self, position: Point) {
        let command = {
            let Some(gesture) = self.resize_gesture.as_ref() else {
                return;
            };
            resize_command(
                &gesture.original_layout,
                &gesture.original_rects,
                gesture.handle,
                gesture.origin,
                position,
            )
        };

        let Some(gesture) = self.resize_gesture.as_ref() else {
            return;
        };
        let mut candidate = gesture.original_layout.clone();
        if let Some(command) = command {
            if let Err(error) = candidate.apply(command, &self.metrics) {
                eprintln!("Dock resize preview rejected: {error}");
            }
        }
        self.layout = candidate;
    }

    fn finish_resize(&mut self, position: Point) {
        let Some(gesture) = self.resize_gesture.take() else {
            return;
        };
        let command = resize_command(
            &gesture.original_layout,
            &gesture.original_rects,
            gesture.handle,
            gesture.origin,
            position,
        );
        self.layout = gesture.original_layout;
        if let Some(command) = command {
            self.apply_command(command);
        }
    }

    fn cancel_resize(&mut self) {
        if let Some(gesture) = self.resize_gesture.take() {
            self.layout = gesture.original_layout;
        }
    }

    fn escape_or_dismiss(&mut self) -> bool {
        if self.resize_gesture.is_some() {
            self.cancel_resize();
            return true;
        }
        if let Some(mut session) = self.drag_session.take() {
            let _ = session.escape();
            self.drag_overlay = None;
            return true;
        }
        if self.menu_request.take().is_some() {
            return true;
        }
        if self.layout.flyout().is_some() {
            self.apply_command(DockCommand::DismissFlyout);
            return true;
        }
        false
    }

    fn render_menu<F>(
        &self,
        request: DockMenuRequest,
        rects: &LayoutRects,
        make_command_handler: &F,
        entity: &gpui::Entity<MainWindow>,
    ) -> Option<Div>
    where
        F: Fn(DockCommand) -> CommandClickHandler,
    {
        let anchor = match request {
            DockMenuRequest::TabOverflow { group } => rects
                .tab_overflows
                .iter()
                .find(|entry| entry.group == group)
                .map(|entry| entry.button_rect)
                .or_else(|| rects.tab_strips.iter().find(|entry| entry.group == group).map(|entry| entry.rect)),
            DockMenuRequest::PanelGroup { group, .. } => rects
                .tab_strips
                .iter()
                .find(|entry| entry.group == group)
                .map(|entry| entry.rect),
        }?;
        let viewport = rects.viewport?;
        let menu_width = 220.0_f32.min(viewport.size.width.max(1.0));
        let left = (anchor.origin.x + anchor.size.width - menu_width)
            .clamp(viewport.origin.x, viewport.origin.x + (viewport.size.width - menu_width).max(0.0));
        let top = (anchor.origin.y + anchor.size.height)
            .clamp(viewport.origin.y, viewport.origin.y + (viewport.size.height - 160.0).max(0.0));

        let heading = match request {
            DockMenuRequest::TabOverflow { .. } => "Tabs",
            DockMenuRequest::PanelGroup { .. } => "Group",
        };
        let mut menu = div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(menu_width))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .p(px(6.0))
            .bg(rgb(self.theme.colors.toolbar_background))
            .border_1()
            .border_color(rgb(self.theme.colors.border))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .text_color(rgb(self.theme.colors.primary_text))
                    .child(heading)
                    .child(
                        div()
                            .id("dock-menu-close")
                            .p(px(4.0))
                            .child("×")
                            .on_click({
                                let entity = entity.clone();
                                Box::new(move |_: &ClickEvent, _: &mut Window, app: &mut App| {
                                    entity.update(app, |window, cx| {
                                        window.dock.menu_request = None;
                                        cx.notify();
                                    });
                                }) as CommandClickHandler
                            }),
                    ),
            );

        let group_id = match request {
            DockMenuRequest::TabOverflow { group }
            | DockMenuRequest::PanelGroup { group, .. } => group,
        };
        let Some(Node::Tabs(tabs)) = self.layout.node(group_id) else {
            return Some(menu);
        };

        match request {
            DockMenuRequest::TabOverflow { group } => {
                for (index, child) in tabs.children.iter().enumerate() {
                    let Some(Node::Panel(panel)) = self.layout.node(*child) else {
                        continue;
                    };
                    let Some(instance) = self.layout.panel_instance(panel.panel_id) else {
                        continue;
                    };
                    let title = panel_title(instance.key.as_str());
                    let label = if index == tabs.active_index {
                        format!("✓ {title}")
                    } else {
                        title.to_owned()
                    };
                    menu = menu.child(menu_item(
                        label,
                        make_command_handler(DockCommand::ActivateTab {
                            group,
                            panel: panel.panel_id,
                        }),
                        self.theme,
                    ));
                }
            }
            DockMenuRequest::PanelGroup { group, panel } => {
                if let Some(panel) = panel {
                    if let Some(instance) = self.layout.panel_instance(panel) {
                        menu = menu.child(menu_item(
                            format!("Close {}", panel_title(instance.key.as_str())),
                            make_command_handler(DockCommand::ClosePanel { panel }),
                            self.theme,
                        ));
                    }
                }
                menu = menu.child(menu_item(
                    "Close group",
                    make_command_handler(DockCommand::CloseGroup { group }),
                    self.theme,
                ));
                menu = menu.child(menu_item(
                    if tabs.collapsed { "Expand group" } else { "Collapse group" },
                    make_command_handler(DockCommand::CollapseGroup {
                        group,
                        collapsed: !tabs.collapsed,
                    }),
                    self.theme,
                ));
            }
        }
        Some(menu)
    }
}

fn resize_command(
    layout: &Layout,
    rects: &LayoutRects,
    handle: ResizeHandle,
    origin: Point,
    position: Point,
) -> Option<DockCommand> {
    let delta_x = position.x - origin.x;
    let delta_y = position.y - origin.y;
    match handle {
        ResizeHandle::OuterColumn { column, side } => {
            let Some(Node::Column(value)) = layout.node(column) else {
                return None;
            };
            let signed_delta = if side == DockSide::Left {
                delta_x
            } else {
                -delta_x
            };
            Some(DockCommand::ResizeColumn {
                column,
                width: value.width + signed_delta,
            })
        }
        ResizeHandle::Splitter {
            owner,
            boundary_index,
            axis: Axis::X,
        } => {
            let side = [DockSide::Left, DockSide::Right].into_iter().find(|side| {
                layout
                    .main_area(*side)
                    .is_some_and(|area| area.root == owner)
            })?;
            Some(DockCommand::ResizeColumnBoundary {
                side,
                boundary_index,
                delta: delta_x,
            })
        }
        ResizeHandle::Splitter {
            owner,
            boundary_index,
            axis: Axis::Y,
        } => {
            let Some(Node::Column(column)) = layout.node(owner) else {
                return None;
            };
            let available_height = column
                .groups
                .iter()
                .filter_map(|group| rects.groups.iter().find(|entry| entry.group == *group))
                .map(|entry| entry.rect.size.height)
                .sum::<f32>();
            (available_height > 0.0).then_some(DockCommand::ResizeGroups {
                column: owner,
                boundary_index,
                delta: delta_y,
                available_height,
            })
        }
        ResizeHandle::FloatingEdge { group, edge } => {
            let start = rects
                .floating_groups
                .iter()
                .find(|entry| entry.group == group)?
                .rect;
            let moves_left = matches!(edge, ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft);
            let moves_right = matches!(edge, ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight);
            let moves_top = matches!(edge, ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight);
            let moves_bottom = matches!(edge, ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight);
            let mut bounds = start;
            if moves_left {
                bounds.origin.x += delta_x;
                bounds.size.width -= delta_x;
            } else if moves_right {
                bounds.size.width += delta_x;
            }
            if moves_top {
                bounds.origin.y += delta_y;
                bounds.size.height -= delta_y;
            } else if moves_bottom {
                bounds.size.height += delta_y;
            }
            bounds.size.width = bounds.size.width.max(1.0);
            bounds.size.height = bounds.size.height.max(1.0);
            Some(DockCommand::ResizeFloatingBounds { group, bounds })
        }
    }
}

fn core_point(position: GpuiPoint<Pixels>) -> Point {
    Point::new(position.x.as_f32(), position.y.as_f32())
}

fn rect_contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.origin.x
        && point.y >= rect.origin.y
        && point.x <= rect.origin.x + rect.size.width
        && point.y <= rect.origin.y + rect.size.height
}

fn rect_area(rect: Rect) -> f32 {
    rect.size.width.max(0.0) * rect.size.height.max(0.0)
}

fn panel_title(key: &str) -> &'static str {
    match key {
        "layers" => "Layers",
        "color" => "Color",
        "properties" => "Properties",
        "history" => "History",
        _ => "Panel",
    }
}

fn panel_icon(key: &str) -> &'static str {
    match key {
        "layers" => "▱",
        "color" => "◉",
        "properties" => "⚙",
        "history" => "↶",
        _ => "▣",
    }
}

fn render_panel(instance: &PanelInstance, theme: Theme) -> AnyElement {
    let title = panel_title(instance.key.as_str());
    let description = match instance.key.as_str() {
        "layers" => "Layer list placeholder",
        "color" => "Color controls placeholder",
        "properties" => "Selected object properties placeholder",
        "history" => "Action history placeholder",
        _ => "Panel content",
    };

    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(12.0))
        .text_color(rgb(theme.colors.primary_text))
        .bg(rgb(theme.colors.group_background))
        .child(title)
        .child(description)
        .into_any_element()
}

fn menu_item(label: impl Into<String>, handler: CommandClickHandler, theme: Theme) -> impl IntoElement {
    let label = label.into();
    div()
        .id(format!("dock-menu-item-{label}"))
        .w_full()
        .p(px(6.0))
        .text_color(rgb(theme.colors.primary_text))
        .bg(rgb(theme.colors.toolbar_background))
        .child(label)
        .on_click(handler)
}
