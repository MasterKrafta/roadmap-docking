use dock_core::{DockSide, Node, Point, Rect, Size};
use gpui::IntoElement;

use crate::components::{ResizeEdgeHandle, ResizeEdgeKind};
use crate::host::DockHost;
use crate::requests::ResizeHandle;
use crate::surface::RenderData;
use crate::tree_query::main_column_ids;

pub(crate) fn render(data: &RenderData<'_>, host: &mut dyn DockHost) -> Vec<gpui::AnyElement> {
    let layout = data.layout;
    let rects = data.rects;
    let theme = data.theme;
    let mut elements = Vec::new();

    for side in [DockSide::Left, DockSide::Right] {
        let columns = main_column_ids(layout, side);
        let column_id = if side == DockSide::Left {
            columns.last().copied()
        } else {
            columns.first().copied()
        };
        let Some(column_id) = column_id else {
            continue;
        };
        if !matches!(layout.node(column_id), Some(Node::Column(_))) {
            continue;
        }
        let Some(column_rect) = rects.columns.iter().find(|entry| entry.column == column_id) else {
            continue;
        };
        let width = theme
            .metrics
            .splitter_hit_zone
            .min(column_rect.rect.size.width);
        let edge_x = if side == DockSide::Left {
            column_rect.rect.origin.x + column_rect.rect.size.width
        } else {
            column_rect.rect.origin.x
        };
        let handle_rect = Rect::new(
            Point::new(edge_x - width / 2.0, column_rect.rect.origin.y),
            Size::new(width, column_rect.rect.size.height),
        );
        elements.push(
            ResizeEdgeHandle {
                id: format!("dock-outer-column-resize-{:?}", column_id),
                rect: handle_rect,
                edge: ResizeEdgeKind::OuterColumn,
                theme: *theme,
                on_resize_start: host.resize_start(ResizeHandle::OuterColumn {
                    column: column_id,
                    side,
                }),
                on_resize_key: Some(host.resize_key(ResizeHandle::OuterColumn {
                    column: column_id,
                    side,
                })),
            }
            .into_any_element(),
        );
    }

    elements
}
