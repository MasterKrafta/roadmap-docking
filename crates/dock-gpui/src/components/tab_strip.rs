use std::collections::{HashMap, HashSet};

use dock_core::{
    DockCommand, FloatingGroupId, Layout, Node, NodeId, PanelId, TabHeaderRect, TabOverflowRect,
    TabStripRect,
};
use gpui::{
    Entity, IntoElement, MouseButton, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, div, img, prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::{DockController, IconSource, PanelRegistry};

use super::tab_overflow::TabOverflow;
use crate::actions::dispatch_command;

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

/// Tab-strip surface and tab headers for one Tabs group.
pub struct TabStripView;

/// One visible tab header, including its close affordance and drag gesture.
pub struct TabHeaderView;

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
        raise_floating: Option<FloatingGroupId>,
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
            .bg(rgb(theme.colors.tab_strip_background));

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
                controller.clone(),
                theme,
                raise_floating,
                middle_click_closes_tabs,
            ));
        }

        if let Some(overflow) = overflow {
            strip_root = strip_root.child(TabOverflow::render(strip.rect, overflow, theme));
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
        controller: Entity<DockController>,
        theme: Theme,
        raise_floating: Option<FloatingGroupId>,
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
            title_row = title_row.child(img(icon).size(px(14.0)).flex_none());
        }

        title_row = title_row.child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(title),
        );

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
            .on_aux_click({
                let controller = controller.clone();
                let panel = header.panel;
                move |event, _, app| {
                    // Singleton constrains opening multiplicity; it does not prevent closing.
                    if middle_click_closes_tabs && event.is_middle_click() {
                        dispatch_command(&controller, DockCommand::ClosePanel { panel }, app);
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
            .on_mouse_down(MouseButton::Left, {
                let controller = controller.clone();
                move |_, _, app| {
                    // The close control stops bubbling to avoid starting a tab drag,
                    // so it raises its owning floating composition explicitly.
                    if let Some(group) = raise_floating {
                        dispatch_command(&controller, DockCommand::RaiseFloating { group }, app);
                    }
                    app.stop_propagation();
                }
            })
            .on_click(move |_, _, app| {
                dispatch_command(
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

        let active_panel = tabs
            .active_child()
            .and_then(|child| match layout.node(child) {
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
