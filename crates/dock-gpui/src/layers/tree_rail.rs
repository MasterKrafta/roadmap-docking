use dock_core::{Node, Point, Rect, Size};
use gpui::{AnyElement, IntoElement};

use crate::components::RailEntry;
use crate::host::DockHost;
use crate::surface::RenderData;

use super::tree::TreeArea;

pub(crate) fn render(
    data: &RenderData<'_>,
    host: &mut dyn DockHost,
    area: TreeArea<'_>,
) -> Vec<AnyElement> {
    let layout = data.layout;
    let rects = area.rects;
    let nodes = area.nodes;
    let theme = data.theme;
    let mut elements = Vec::new();

    for column_rect in rects.columns.iter().filter(|entry| nodes.contains(&entry.column)) {
        let Some(Node::Column(column)) = layout.node(column_rect.column) else { continue; };
        if !column.mode.is_rail() { continue; }
        let Ok(items) = layout.column_display_order(column_rect.column) else { continue; };
        let selected_flyout = layout.flyout()
            .filter(|flyout| flyout.column == column_rect.column)
            .map(|flyout| flyout.panel);
        let column_bounds = area.column_rect(column_rect.column, column_rect.rect);
        let visible_bottom = column_bounds.origin.y + column_bounds.size.height;

        for item in items {
            let Some(header) = rects.tab_headers.iter().find(|header| {
                header.group == item.group && header.panel == item.panel
            }) else { continue; };
            let header_bounds = area.rect(header.rect);
            if header_bounds.origin.y >= visible_bottom { break; }
            let height = header_bounds.size.height.min(visible_bottom - header_bounds.origin.y);
            if height <= 0.0 { continue; }
            let Some(instance) = layout.panel_instance(item.panel) else { continue; };

            let active_in_model = matches!(
                layout.node(item.group),
                Some(Node::Tabs(tabs)) if crate::tree_query::active_panel(layout, tabs) == Some(item.panel)
            );
            elements.push(
                RailEntry {
                    id: format!("dock-rail-{:?}-{:?}", column_rect.column, item.panel),
                    rect: Rect::new(header_bounds.origin, Size::new(header_bounds.size.width, height)),
                    mode: column.mode,
                    icon: host.panel_icon(instance).to_owned(),
                    title: host.panel_title(instance).to_string(),
                    active: selected_flyout.map_or(active_in_model, |panel| panel == item.panel),
                    theme: *theme,
                    on_open: host.command(dock_core::DockCommand::OpenFlyout {
                        column: column_rect.column,
                        panel: item.panel,
                    }),
                }.into_any_element(),
            );
        }
    }
    elements
}
