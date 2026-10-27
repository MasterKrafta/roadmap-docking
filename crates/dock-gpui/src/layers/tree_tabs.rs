use dock_core::{ColumnMode, DragHandle, DragPayload, Node, NodeId, Point, Rect, Size};
use gpui::{AnyElement, IntoElement};

use crate::components::{ColumnModeButton, GroupMenuButton, PanelTab, TabOverflowButton};
use crate::dock::{activate_tab, collapse_group, cycle_column_mode};
use crate::host::DockHost;
use crate::labels::tab_header_label;
use crate::requests::DockMenuRequest;
use crate::surface::RenderData;
use crate::tree_query::active_panel;

use super::tree::TreeArea;

pub(crate) fn render(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    area: TreeArea<'_>,
    rail_groups: &std::collections::HashSet<NodeId>,
) -> Vec<AnyElement> {
    let layout = data.layout;
    let theme = data.theme;
    let rects = area.rects;
    let nodes = area.nodes;
    let mut elements = Vec::new();

    for header in rects.tab_headers.iter().filter(|entry| {
        nodes.contains(&entry.group) && !rail_groups.contains(&entry.group)
    }) {
        let collapsed = matches!(layout.node(header.group), Some(Node::Tabs(tabs)) if tabs.collapsed);
        let label = layout.panel_instance(header.panel)
            .map(|instance| host.panel_title(instance).to_string())
            .unwrap_or_else(|| "Missing panel".to_owned());
        let display_label = tab_header_label(&label, collapsed, header.index);
        let active = matches!(
            layout.node(header.group),
            Some(Node::Tabs(tabs)) if active_panel(layout, tabs) == Some(header.panel)
        );

        elements.push(
            PanelTab {
                id: format!("dock-tab-{:?}-{:?}", header.group, header.panel),
                rect: area.rect(header.rect),
                label: display_label,
                active,
                theme: *theme,
                on_activate: host.command(activate_tab(header.group, header.panel)),
                on_collapse_toggle: host.command(collapse_group(header.group, !collapsed)),
                on_drag_start: host.drag_start(
                    DragPayload::Panel { panel: header.panel },
                    DragHandle::PanelTab,
                ),
            }.into_any_element(),
        );
    }

    for overflow in rects.tab_overflows.iter().filter(|entry| nodes.contains(&entry.group)) {
        elements.push(
            TabOverflowButton {
                group: overflow.group,
                rect: area.rect(overflow.button_rect),
                theme: *theme,
                on_open: host.menu(DockMenuRequest::TabOverflow { group: overflow.group }),
            }.into_any_element(),
        );
    }

    for strip in rects.tab_strips.iter().filter(|entry| {
        nodes.contains(&entry.group) && !rail_groups.contains(&entry.group)
    }) {
        let Some(Node::Tabs(tabs)) = layout.node(strip.group) else { continue; };
        let strip_bounds = area.rect(strip.rect);
        let button_width = theme.metrics.group_control_button_width.min(strip.rect.size.width);
        let mode_button_column = layout.nodes().find_map(|(column_id, node)| match node {
            Node::Column(column)
                if column.groups.first().copied() == Some(strip.group)
                    && column.mode == ColumnMode::Expanded =>
            {
                Some((column_id, column.mode))
            }
            _ => None,
        });
        let mode_button_width = if mode_button_column.is_some() {
            theme
                .metrics
                .group_control_button_width
                .min((strip.rect.size.width - button_width).max(0.0))
        } else {
            0.0
        };
        if let Some((column_id, mode)) = mode_button_column
            && mode_button_width > 0.0
        {
            elements.push(
                ColumnModeButton {
                    column: column_id,
                    mode,
                    rect: Rect::new(
                        Point::new(
                            strip_bounds.origin.x + strip_bounds.size.width
                                - button_width - mode_button_width,
                            strip_bounds.origin.y,
                        ),
                        Size::new(mode_button_width, strip_bounds.size.height),
                    ),
                    theme: *theme,
                    on_toggle: host.command(cycle_column_mode(column_id, mode)),
                }
                .into_any_element(),
            );
        }
        let menu_rect = Rect::new(
            Point::new(strip_bounds.origin.x + strip_bounds.size.width - button_width, strip_bounds.origin.y),
            Size::new(button_width, strip_bounds.size.height),
        );
        let panel = active_panel(layout, tabs);
        elements.push(
            GroupMenuButton {
                group: strip.group,
                panel,
                rect: menu_rect,
                theme: *theme,
                on_open: host.menu(DockMenuRequest::PanelGroup { group: strip.group, panel }),
            }.into_any_element(),
        );
    }

    elements
}
