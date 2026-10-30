use std::{collections::{HashMap, HashSet}, rc::Rc};

use dock_core::{
    DockCommand, DragHandle, DragPayload, DragState, GroupRect, Layout, Node, NodeId, PanelId,
    Point, TabHeaderRect, TabOverflowRect, TabStripRect,
};
use gpui::{
    App, Entity, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, SharedString, Styled, div, img, px, rgb,
    prelude::InteractiveElement as _,
};
use ui::Theme;

use crate::{DockController, IconSource, PanelRegistry};

pub(crate) type GroupMenuToggle = Rc<dyn Fn(NodeId, &mut App)>;
pub(crate) type TabOverflowToggle = Rc<dyn Fn(NodeId, &mut App)>;

#[derive(Clone)]
pub(crate) struct TabPresentation {
    pub panel: PanelId,
    pub title: SharedString,
    pub icon: Option<SharedString>,
    pub index: usize,
}

#[derive(Clone, Default)]
pub(crate) struct GroupPresentation {
    pub collapsed: bool,
    pub active_panel: Option<PanelId>,
    pub tabs: Vec<TabPresentation>,
}

/// The group component: grip, tab strip, controls, and group context menu.
pub struct GroupView;

/// Tab-strip surface and tab headers for one Tabs group.
pub struct TabStripView;

/// One visible tab header, including its close affordance and drag gesture.
pub struct TabHeaderView;

/// Overflow affordance and popup list for tabs omitted by Core geometry.
pub struct TabOverflow;

impl GroupView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        group: GroupRect,
        strip: TabStripRect,
        headers: &[TabHeaderRect],
        overflow: Option<TabOverflowRect>,
        presentation: &GroupPresentation,
        content_origin: Point,
        metrics: dock_core::DockMetrics,
        reserved_grip_left: f32,
        controller: Entity<DockController>,
        theme: Theme,
        menu_open: bool,
        toggle_menu: GroupMenuToggle,
        overflow_open: bool,
        toggle_overflow: TabOverflowToggle,
        middle_click_closes_tabs: bool,
    ) -> impl IntoElement {
        let left = group.rect.origin.x - content_origin.x;
        let top = group.rect.origin.y - content_origin.y;
        let header_height = if presentation.collapsed {
            0.0
        } else {
            metrics.header_height.min(group.rect.size.height)
        };
        let control_width = metrics
            .group_control_button_width
            .min(group.rect.size.width / 3.0)
            .max(1.0);

        let mut root = div()
            .id(format!("dock-group-{:?}", group.group))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(group.rect.size.width))
            .h(px(group.rect.size.height))
            .bg(rgb(theme.colors.group_frame_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border));

        if header_height > 0.0 {
            let grip_controller = controller.clone();
            let grip_group = group.group;
            let grip = div()
                .id(format!("group-grip-{:?}", group.group))
                .absolute()
                .top_0()
                .left(px(reserved_grip_left))
                .w(px(
                    (group.rect.size.width - reserved_grip_left - 2.0 * control_width).max(0.0),
                ))
                .h(px(header_height))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(rgb(theme.colors.group_grip_background))
                .text_color(rgb(theme.colors.primary_text))
                .child("⠿")
                .on_mouse_down(MouseButton::Left, move |event, _, app| {
                    begin_group_drag(&grip_controller, grip_group, event, app);
                })
                .on_mouse_move({
                    let controller = controller.clone();
                    move |event, _, app| update_group_drag(&controller, event, app)
                })
                .on_mouse_up({
                    let controller = controller.clone();
                    move |event, _, app| finish_group_drag(&controller, event, app)
                })
                .on_mouse_up_out(MouseButton::Left, {
                    let controller = controller.clone();
                    move |event, _, app| finish_group_drag(&controller, event, app)
                });
            root = root.child(grip);
        }

        root = root.child(TabStripView::render(
            group.group,
            group.rect,
            strip,
            headers,
            overflow,
            presentation,
            controller.clone(),
            theme,
            overflow_open,
            toggle_overflow,
            middle_click_closes_tabs,
        ));

        // Collapsed groups have no header rect, so their controls overlay the
        // trailing edge of the remaining tab strip to keep Expand/Menu reachable.
        let controls_top = if presentation.collapsed {
            strip.rect.origin.y - group.rect.origin.y
        } else {
            0.0
        };
        let controls_height = if presentation.collapsed {
            strip.rect.size.height
        } else {
            header_height
        };

        let menu_toggle = toggle_menu.clone();
        let menu_button = div()
            .id(format!("group-menu-button-{:?}", group.group))
            .absolute()
            .top(px(controls_top))
            .right(px(0.0))
            .w(px(control_width))
            .h(px(controls_height))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.group_menu_button))
            .text_color(rgb(theme.colors.primary_text))
            .child("⋮")
            .on_click(move |_, _, app| (menu_toggle)(group.group, app));
        root = root.child(menu_button);

        let collapse_controller = controller.clone();
        let collapse_group = group.group;
        let collapse_to = !presentation.collapsed;
        let collapse_button = div()
            .id(format!("group-collapse-button-{:?}", group.group))
            .absolute()
            .top(px(controls_top))
            .right(px(control_width))
            .w(px(control_width))
            .h(px(controls_height))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.group_menu_button))
            .text_color(rgb(theme.colors.primary_text))
            .child(if presentation.collapsed { "＋" } else { "−" })
            .on_click(move |_, _, app| {
                dispatch_group_command(
                    &collapse_controller,
                    DockCommand::CollapseGroup {
                        group: collapse_group,
                        collapsed: collapse_to,
                    },
                    app,
                );
            });
        root = root.child(collapse_button);

        if menu_open {
            let menu_top = if presentation.collapsed {
                controls_top + strip.rect.size.height
            } else {
                header_height
            };
            let mut menu = div()
                .id(format!("group-context-menu-{:?}", group.group))
                .absolute()
                .top(px(menu_top))
                .right(px(4.0))
                .w(px(theme.metrics.popup_menu_width.min(group.rect.size.width)))
                .flex()
                .flex_col()
                .border_1()
                .border_color(rgb(theme.colors.popup_menu_border))
                .bg(rgb(theme.colors.popup_menu_background));

            menu = menu.child(menu_entry(
                "Закрыть группу",
                DockCommand::CloseGroup { group: group.group },
                group.group,
                controller.clone(),
                toggle_menu.clone(),
                theme,
            ));

            if let Some(panel) = presentation.active_panel {
                menu = menu.child(menu_entry(
                    "Закрыть вкладку",
                    DockCommand::ClosePanel { panel },
                    group.group,
                    controller.clone(),
                    toggle_menu.clone(),
                    theme,
                ));
            }

            menu = menu.child(menu_entry(
                if presentation.collapsed { "Развернуть" } else { "Свернуть" },
                DockCommand::CollapseGroup {
                    group: group.group,
                    collapsed: !presentation.collapsed,
                },
                group.group,
                controller.clone(),
                toggle_menu.clone(),
                theme,
            ));

            menu = menu.child(menu_entry(
                "Вынести в плавающее окно",
                DockCommand::Float {
                    node: group.group,
                    bounds: group.rect,
                },
                group.group,
                controller,
                toggle_menu,
                theme,
            ));
            root = root.child(menu);
        }

        root
    }
}

impl TabStripView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        group_id: NodeId,
        group_rect: dock_core::Rect,
        strip: TabStripRect,
        headers: &[TabHeaderRect],
        overflow: Option<TabOverflowRect>,
        presentation: &GroupPresentation,
        controller: Entity<DockController>,
        theme: Theme,
        overflow_open: bool,
        toggle_overflow: TabOverflowToggle,
        middle_click_closes_tabs: bool,
    ) -> impl IntoElement {
        let mut strip_root = div()
            .id(format!("tab-strip-{:?}", group_id))
            .absolute()
            .left(px(strip.rect.origin.x - group_rect.origin.x))
            .top(px(strip.rect.origin.y - group_rect.origin.y))
            .w(px(strip.rect.size.width))
            .h(px(strip.rect.size.height))
            .flex()
            .items_stretch()
            .bg(rgb(theme.colors.tab_strip_background))
            .on_click({
                let controller = controller.clone();
                let collapsed = presentation.collapsed;
                move |event, _, app| {
                    if event.click_count() > 1 {
                        dispatch_group_command(
                            &controller,
                            DockCommand::CollapseGroup {
                                group: group_id,
                                collapsed: !collapsed,
                            },
                            app,
                        );
                    }
                }
            });

        // Inactive tabs are painted first so the active tab remains in front.
        let mut ordered_headers = headers.to_vec();
        ordered_headers.sort_by_key(|header| presentation.active_panel == Some(header.panel));
        for header in ordered_headers {
            let tab = presentation
                .tabs
                .iter()
                .find(|tab| tab.panel == header.panel && tab.index == header.index);
            let title = tab
                .map(|tab| tab.title.clone())
                .unwrap_or_else(|| format!("Panel {}", header.index + 1).into());
            let icon = tab.and_then(|tab| tab.icon.clone());
            let active = presentation.active_panel == Some(header.panel);

            strip_root = strip_root.child(TabHeaderView::render(
                group_id,
                header,
                strip.rect,
                title,
                icon,
                active,
                presentation.collapsed,
                controller.clone(),
                theme,
                middle_click_closes_tabs,
            ));
        }

        if let Some(overflow) = overflow {
            let hidden_tabs = overflow
                .hidden_tabs
                .iter()
                .filter_map(|panel| {
                    presentation
                        .tabs
                        .iter()
                        .find(|tab| tab.panel == *panel)
                        .cloned()
                })
                .collect::<Vec<_>>();
            strip_root = strip_root.child(TabOverflow::render(
                group_rect,
                strip.rect,
                overflow,
                &hidden_tabs,
                presentation.active_panel,
                controller,
                theme,
                overflow_open,
                toggle_overflow,
            ));
        }

        strip_root
    }
}

impl TabHeaderView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        group_id: NodeId,
        header: TabHeaderRect,
        strip_rect: dock_core::Rect,
        title: SharedString,
        icon: Option<SharedString>,
        active: bool,
        collapsed: bool,
        controller: Entity<DockController>,
        theme: Theme,
        middle_click_closes_tabs: bool,
    ) -> impl IntoElement {
        let left = header.rect.origin.x - strip_rect.origin.x;
        let top = header.rect.origin.y - strip_rect.origin.y;
        let width = header.rect.size.width.max(0.0);
        let height = header.rect.size.height.max(0.0);
        let close_width = 18.0_f32.min(width.max(0.0));
        let hover_group = format!("dock-tab-{:?}-{:?}", group_id, header.panel);

        let mut title_row = div()
            .absolute()
            .left(px(6.0))
            .right(px(close_width))
            .top_0()
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size));

        if let Some(icon) = icon {
            title_row = title_row.child(
                img(icon)
                    .size(px(14.0))
                    .flex_none(),
            );
        }

        title_row = title_row.child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(title),
        );

        let tab_controller = controller.clone();
        let tab_group = group_id;
        let tab_panel = header.panel;
        let tab = div()
            .id(format!("tab-{:?}-{:?}", group_id, header.panel))
            .group(hover_group.clone())
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .overflow_hidden()
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .bg(rgb(if active {
                theme.colors.panel_tab_active
            } else {
                theme.colors.panel_tab_inactive
            }))
            .text_color(rgb(theme.colors.primary_text))
            .cursor_pointer()
            .child(title_row)
            .on_mouse_down(MouseButton::Left, {
                let controller = controller.clone();
                move |event, _, app| begin_panel_drag(&controller, tab_panel, event, app)
            })
            .on_mouse_move({
                let controller = controller.clone();
                move |event, _, app| update_panel_drag(&controller, event, app)
            })
            .on_mouse_up({
                let controller = controller.clone();
                move |event, _, app| finish_panel_drag(&controller, event, app)
            })
            .on_mouse_up_out(MouseButton::Left, {
                let controller = controller.clone();
                move |event, _, app| finish_panel_drag(&controller, event, app)
            })
            .on_click(move |event, _, app| {
                if event.standard_click() {
                    if event.click_count() > 1 {
                        dispatch_group_command(
                            &tab_controller,
                            DockCommand::CollapseGroup {
                                group: tab_group,
                                collapsed: !collapsed,
                            },
                            app,
                        );
                    } else {
                        dispatch_group_command(
                            &tab_controller,
                            DockCommand::ActivateTab {
                                group: tab_group,
                                panel: tab_panel,
                            },
                            app,
                        );
                    }
                }
                app.stop_propagation();
            })
            .on_aux_click({
                let controller = controller.clone();
                let panel = header.panel;
                move |event, _, app| {
                    // Singleton constrains opening multiplicity; it does not prevent closing.
                    if middle_click_closes_tabs && event.is_middle_click() {
                        dispatch_group_command(
                            &controller,
                            DockCommand::ClosePanel { panel },
                            app,
                        );
                        app.stop_propagation();
                    }
                }
            });

        let close_controller = controller.clone();
        let close_panel = header.panel;
        let close_button = div()
            .id(format!("tab-close-{:?}-{:?}", group_id, header.panel))
            .absolute()
            .top_0()
            .right_0()
            .w(px(close_width))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .opacity(0.0)
            .group_hover(hover_group, |style| style.opacity(1.0))
            .hover(|style| style.bg(rgb(theme.colors.icon_button_hover)))
            .text_color(rgb(theme.colors.primary_text))
            .child("×")
            .on_mouse_down(MouseButton::Left, |_, _, app| {
                // Do not let a close-button press also begin a tab drag.
                app.stop_propagation();
            })
            .on_click(move |_, _, app| {
                dispatch_group_command(
                    &close_controller,
                    DockCommand::ClosePanel { panel: close_panel },
                    app,
                );
                app.stop_propagation();
            });

        // Singleton constrains opening multiplicity; it deliberately does not block closing.
        tab.child(close_button)
    }
}

impl TabOverflow {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        group_rect: dock_core::Rect,
        strip_rect: dock_core::Rect,
        overflow: TabOverflowRect,
        hidden_tabs: &[TabPresentation],
        active_panel: Option<PanelId>,
        controller: Entity<DockController>,
        theme: Theme,
        menu_open: bool,
        toggle_overflow: TabOverflowToggle,
    ) -> impl IntoElement {
        let left = overflow.button_rect.origin.x - strip_rect.origin.x;
        let top = overflow.button_rect.origin.y - strip_rect.origin.y;
        let button_width = overflow.button_rect.size.width;
        let button_height = overflow.button_rect.size.height;
        let mut root = div()
            .id(format!("tab-overflow-{:?}", overflow.group))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(button_width))
            .h(px(button_height));

        let button_toggle = toggle_overflow.clone();
        let button = div()
            .w_full()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.tab_overflow_button))
            .hover(|style| style.bg(rgb(theme.colors.tab_overflow_button_hover)))
            .text_color(rgb(theme.colors.primary_text))
            .child("»»")
            .on_click(move |_, _, app| {
                (button_toggle)(overflow.group, app);
                app.stop_propagation();
            });
        root = root.child(button);

        if menu_open {
            let menu_width = theme.metrics.popup_menu_width.min(group_rect.size.width).max(1.0);
            let popup_left = (left + button_width - menu_width)
                .clamp(0.0, (strip_rect.size.width - menu_width).max(0.0));
            let mut menu = div()
                .id(format!("tab-overflow-menu-{:?}", overflow.group))
                .absolute()
                .left(px(popup_left - left))
                .top(px(button_height))
                .w(px(menu_width))
                .flex()
                .flex_col()
                .border_1()
                .border_color(rgb(theme.colors.popup_menu_border))
                .bg(rgb(theme.colors.popup_menu_background));

            for tab in hidden_tabs {
                let is_active = active_panel == Some(tab.panel);
                menu = menu.child(overflow_menu_entry(
                    tab.clone(),
                    is_active,
                    overflow.group,
                    controller.clone(),
                    toggle_overflow.clone(),
                    theme,
                ));
            }
            root = root.child(menu);
        }

        root
    }
}

fn overflow_menu_entry(
    tab: TabPresentation,
    active: bool,
    group: NodeId,
    controller: Entity<DockController>,
    toggle_overflow: TabOverflowToggle,
    theme: Theme,
) -> impl IntoElement {
    let mut row = div()
        .w_full()
        .h(px(theme.metrics.popup_menu_row_height))
        .flex()
        .items_center()
        .px(px(theme.metrics.popup_menu_horizontal_padding))
        .cursor_pointer()
        .text_color(rgb(theme.colors.primary_text))
        .bg(rgb(if active {
            theme.colors.popup_menu_selected
        } else {
            theme.colors.popup_menu_background
        }))
        .hover(|style| style.bg(rgb(theme.colors.popup_menu_hover)));

    if let Some(icon) = tab.icon.clone() {
        row = row.child(img(icon).size(px(14.0)).flex_none());
    }
    row = row.child(
        div()
            .flex_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(tab.title.clone()),
    );

    row.on_click(move |_, _, app| {
        dispatch_group_command(
            &controller,
            DockCommand::ActivateTab {
                group,
                panel: tab.panel,
            },
            app,
        );
        (toggle_overflow)(group, app);
        app.stop_propagation();
    })
}

fn menu_entry(
    label: &'static str,
    command: DockCommand,
    group: NodeId,
    controller: Entity<DockController>,
    toggle_menu: GroupMenuToggle,
    theme: Theme,
) -> impl IntoElement {
    div()
        .w_full()
        .h(px(theme.metrics.popup_menu_row_height))
        .flex()
        .items_center()
        .px(px(theme.metrics.popup_menu_horizontal_padding))
        .cursor_pointer()
        .text_color(rgb(theme.colors.primary_text))
        .child(label)
        .on_click(move |_, _, app| {
            dispatch_group_command(&controller, command.clone(), app);
            (toggle_menu)(group, app);
        })
}

pub(crate) fn dispatch_group_command(
    controller: &Entity<DockController>,
    command: DockCommand,
    app: &mut App,
) {
    if let Err(error) = controller.update(app, |controller, controller_cx| {
        controller.dispatch_in(command, controller_cx)
    }) {
        tracing::warn!(%error, "group command failed");
    }
}

fn event_point(x: gpui::Pixels, y: gpui::Pixels) -> Point {
    Point::new(x.as_f32(), y.as_f32())
}

fn begin_group_drag(
    controller: &Entity<DockController>,
    group: NodeId,
    event: &MouseDownEvent,
    app: &mut App,
) {
    let position = event_point(event.position.x, event.position.y);
    if let Err(error) = controller.update(app, |controller, _| {
        controller.drag_mut().session_mut().pointer_down_with_handle(
            DragPayload::Group { group },
            DragHandle::GroupGrip,
            position,
        )
    }) {
        tracing::debug!(%error, "could not begin group drag gesture");
    }
}

fn update_group_drag(
    controller: &Entity<DockController>,
    event: &MouseMoveEvent,
    app: &mut App,
) {
    if !event.dragging() {
        return;
    }
    let position = event_point(event.position.x, event.position.y);
    let _ = controller.update(app, |controller, _| {
        let session = controller.drag_mut().session_mut();
        if matches!(
            session.state(),
            DragState::Pressed { .. } | DragState::Dragging { .. }
        ) {
            let _ = session.pointer_move(position);
        }
    });
}

fn finish_group_drag(
    controller: &Entity<DockController>,
    event: &MouseUpEvent,
    app: &mut App,
) {
    let position = event_point(event.position.x, event.position.y);
    let _ = controller.update(app, |controller, _| {
        let session = controller.drag_mut().session_mut();
        if matches!(
            session.state(),
            DragState::Pressed { .. } | DragState::Dragging { .. }
        ) {
            let _ = session.pointer_up(position, None);
            let _ = session.reset();
        }
    });
}


fn begin_panel_drag(
    controller: &Entity<DockController>,
    panel: PanelId,
    event: &MouseDownEvent,
    app: &mut App,
) {
    let position = event_point(event.position.x, event.position.y);
    if let Err(error) = controller.update(app, |controller, _| {
        controller
            .drag_mut()
            .session_mut()
            .pointer_down(DragPayload::Panel { panel }, position)
    }) {
        tracing::debug!(%error, "could not begin tab drag gesture");
    }
}

fn update_panel_drag(
    controller: &Entity<DockController>,
    event: &MouseMoveEvent,
    app: &mut App,
) {
    if !event.dragging() {
        return;
    }
    let position = event_point(event.position.x, event.position.y);
    let _ = controller.update(app, |controller, _| {
        let session = controller.drag_mut().session_mut();
        if matches!(
            session.state(),
            DragState::Pressed { .. } | DragState::Dragging { .. }
        ) {
            let _ = session.pointer_move(position);
        }
    });
}

fn finish_panel_drag(
    controller: &Entity<DockController>,
    event: &MouseUpEvent,
    app: &mut App,
) {
    let position = event_point(event.position.x, event.position.y);
    let _ = controller.update(app, |controller, _| {
        let session = controller.drag_mut().session_mut();
        if matches!(
            session.state(),
            DragState::Pressed { .. } | DragState::Dragging { .. }
        ) {
            let _ = session.pointer_up(position, None);
            let _ = session.reset();
        }
    });
}

/// Builds presentation metadata for Tabs nodes without retaining another tree.
pub(crate) fn collect_group_presentations(
    layout: &Layout,
    registry: &PanelRegistry,
    node_ids: &HashSet<NodeId>,
) -> HashMap<NodeId, GroupPresentation> {
    let mut groups = HashMap::new();
    for group in node_ids {
        let Some(Node::Tabs(tabs)) = layout.node(*group) else {
            continue;
        };

        let tab_presentations = tabs
            .children
            .iter()
            .enumerate()
            .filter_map(|(index, child)| {
                let Some(Node::Panel(panel)) = layout.node(*child) else {
                    return None;
                };
                let instance = layout.panel_instance(panel.panel_id)?;
                let descriptor = registry.get(&instance.key);
                let title = descriptor
                    .map(|descriptor| descriptor.title().clone())
                    .unwrap_or_else(|| instance.key.to_string().into());
                let icon = descriptor.and_then(|descriptor| match descriptor.icon() {
                    IconSource::Asset(path) => Some(path.clone()),
                    IconSource::None => None,
                });
                Some(TabPresentation {
                    panel: panel.panel_id,
                    title,
                    icon,
                    index,
                })
            })
            .collect();

        let active_panel = tabs.active_child().and_then(|child| match layout.node(child) {
            Some(Node::Panel(panel)) => Some(panel.panel_id),
            _ => None,
        });

        groups.insert(
            *group,
            GroupPresentation {
                collapsed: tabs.collapsed,
                active_panel,
                tabs: tab_presentations,
            },
        );
    }
    groups
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use dock_core::{Layout, NodeId, Profile};
    use slotmap::Key;

    use super::collect_group_presentations;

    #[test]
    fn empty_layout_has_no_group_presentations() {
        let groups = collect_group_presentations(
            &Layout::new(Profile::Photoshop),
            &crate::PanelRegistry::new(),
            &HashSet::<NodeId>::new(),
        );
        assert!(groups.is_empty());
    }

    #[test]
    fn presentation_collection_ignores_non_tabs_nodes() {
        let groups = collect_group_presentations(
            &Layout::new(Profile::Generic),
            &crate::PanelRegistry::new(),
            &HashSet::from([NodeId::null()]),
        );
        assert!(groups.is_empty());
    }
}
