use dock_core::{
    create_initial_layout, DragDropAction, DragDropRequest, DragHandle, DragPayload, DragSession,
    DragState, DockCommand, DockMetrics, DropSource, DropTarget, FlyoutState, Layout, LayoutRects,
    Node, NodeId, PanelId, PanelKey, Point, Profile, Rect, Size,
};
use gpui::{
    App, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, px, rgb,
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

/// The app is the only owner of the live Layout and applies adapter-returned commands.
pub(crate) struct MainWindow {
    layout: Layout,
    theme: Theme,
    metrics: DockMetrics,
    panels: PanelRegistry,
    menu: Option<DockMenuState>,
    drag_session: DragSession,
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
            menu: None,
            drag_session,
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
    ) {
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
                self.layout
                    .resolve_drop(position, source, &rects, &self.metrics)
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
                self.layout
                    .resolve_drop(position, source, rects, &self.metrics)
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
                            + overflow.button_rect.size.height,
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
                                + self.theme.metrics.column_header_height,
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
                    .child(format!("Dock geometry error: {error:?}"));
            }
        };

        let drag_viewport = dock_viewport;
        let move_weak = cx.weak_entity();
        let release_weak = cx.weak_entity();
        let mut root = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.window_background))
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
            .capture_any_mouse_up(move |event, _window, app| {
                let position = Point::new(
                    event.position.x.as_f32(),
                    event.position.y.as_f32() - MENU_BAR_HEIGHT,
                );
                let weak = release_weak.clone();
                let _ = weak.update(app, |view, cx| {
                    view.finish_drag_at(position, drag_viewport, cx);
                });
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
        let dock_view = dock_gpui::render(
            &self.layout,
            &rects,
            theme,
            |instance| panel_registry.render(instance, theme),
            |instance| panel_registry.title(instance),
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
                Box::new(move |event, _window: &mut Window, app: &mut App| {
                    let position = Point::new(
                        event.position.x.as_f32(),
                        event.position.y.as_f32() - MENU_BAR_HEIGHT,
                    );
                    let _ = weak.update(app, |view, cx| {
                        view.begin_drag(payload, handle, position);
                    });
                }) as dock_gpui::DragStartHandler
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

        root
    }
}
