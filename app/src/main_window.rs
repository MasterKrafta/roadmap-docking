use dock_core::{
    create_initial_layout, DockCommand, DockMetrics, FlyoutState, Layout, LayoutRects, Node, NodeId,
    PanelId, PanelKey, Point, Profile, Rect, Size,
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
}

impl MainWindow {
    pub(crate) fn new(theme: Theme) -> Result<Self, String> {
        let panels = PanelRegistry::default();
        let metrics = dock_gpui::dock_metrics_from_theme(&theme);
        let descriptors = panels.core_descriptors();
        let layout = create_initial_layout(Profile::Photoshop, &descriptors, metrics)
            .map_err(str::to_owned)?;

        Ok(Self {
            layout,
            theme,
            metrics,
            panels,
            menu: None,
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

        let mut root = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .bg(rgb(self.theme.colors.window_background));

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

        let panel_registry = &self.panels;
        let theme = &self.theme;
        let command_weak = cx.weak_entity();
        let menu_weak = cx.weak_entity();
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
