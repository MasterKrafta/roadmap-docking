use crate::geometry::Rect;
use crate::ids::{FloatingGroupId, NodeId, PanelId};
use crate::model::DockSide;

use super::{
    ColumnRect, DockAreaRect, FloatingGroupRect, GroupRect, ResizeEdge, SplitterKind, TabStripRect,
};

/// Rectangles produced by layout geometry, all expressed in logical pixels.
///
/// This is a data-only result type: it does not depend on GPUI and does not round
/// coordinates to physical pixels. Platform adapters perform pixel rounding when
/// painting or forwarding input events.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayoutRects {
    /// Protected document/canvas region; it receives the remaining window width.
    pub canvas: Rect,
    /// Full-height rectangles for left/right dock areas.
    pub dock_areas: Vec<DockAreaRect>,
    /// Full-height rectangles for columns, including their shared headers.
    pub columns: Vec<ColumnRect>,
    /// Rectangles for stacked dock groups, including tab strips and content.
    pub groups: Vec<GroupRect>,
    /// Rectangles for complete tab strips.
    pub tab_strips: Vec<TabStripRect>,
    /// Floating groups, ordered bottom-to-top by z_order and positioned over dock rectangles.
    pub floating_groups: Vec<FloatingGroupRect>,
    /// Content rectangles for panel instances.
    pub panels: Vec<PanelRect>,
    /// Tab-header rectangles, in tab-strip order for each group.
    pub tab_headers: Vec<TabHeaderRect>,
    /// Headers for dock columns.
    pub column_headers: Vec<ColumnHeaderRect>,
    /// Splitter hit regions. Their order is kept stable for the associated hit-test indices.
    pub splitters: Vec<SplitterRect>,
    /// Title-bar regions for floating groups.
    pub floating_titles: Vec<FloatingTitleRect>,
    /// Resizable edge/corner regions for floating groups.
    pub floating_resize_edges: Vec<FloatingResizeEdgeRect>,
}

/// Logical rectangle for a panel's content region.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PanelRect {
    pub panel: PanelId,
    pub bounds: Rect,
}

/// Logical rectangle for one tab header.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TabHeaderRect {
    pub group: NodeId,
    pub index: usize,
    pub bounds: Rect,
}

/// Logical rectangle for a dock-column header.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnHeaderRect {
    pub side: DockSide,
    pub index: usize,
    pub bounds: Rect,
}

/// Logical hit rectangle for one splitter.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SplitterRect {
    pub kind: SplitterKind,
    /// Zero-based index used by the matching HitResult::Splitter.
    pub index: usize,
    pub bounds: Rect,
}

/// Logical rectangle for a floating group's title bar.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingTitleRect {
    pub floating: FloatingGroupId,
    pub bounds: Rect,
}

/// Logical resize hit rectangle for one floating-group edge or corner.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingResizeEdgeRect {
    pub floating: FloatingGroupId,
    pub edge: ResizeEdge,
    pub bounds: Rect,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DockSide, FloatingGroupId, NodeId, PanelId, Point, Size};

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect::new(Point::new(x, y), Size::new(width, height))
    }

    #[test]
    fn layout_rects_default_to_empty_collections_and_zero_canvas() {
        let rects = LayoutRects::default();
        assert_eq!(rects.canvas, Rect::default());
        assert!(rects.dock_areas.is_empty());
        assert!(rects.columns.is_empty());
        assert!(rects.groups.is_empty());
        assert!(rects.tab_strips.is_empty());
        assert!(rects.floating_groups.is_empty());
        assert!(rects.panels.is_empty());
        assert!(rects.tab_headers.is_empty());
        assert!(rects.column_headers.is_empty());
        assert!(rects.splitters.is_empty());
        assert!(rects.floating_titles.is_empty());
        assert!(rects.floating_resize_edges.is_empty());
    }

    #[test]
    fn layout_rects_preserve_region_metadata_and_logical_bounds() {
        let bounds = rect(12.5, 24.5, 240.0, 80.0);
        let panel = PanelId::null();
        let group = NodeId::null();
        let floating = FloatingGroupId::null();
        let rects = LayoutRects {
            canvas: rect(0.0, 0.0, 1280.0, 720.0),
            dock_areas: vec![DockAreaRect {
                side: DockSide::Right,
                bounds: rect(1000.0, 0.0, 280.0, 720.0),
            }],
            columns: vec![ColumnRect {
                side: DockSide::Right,
                index: 0,
                bounds: rect(1000.0, 0.0, 280.0, 720.0),
                vertical_overflow: false,
            }],
            groups: vec![GroupRect { group, bounds }],
            tab_strips: vec![TabStripRect { group, bounds }],
            floating_groups: vec![],
            panels: vec![PanelRect { panel, bounds }],
            tab_headers: vec![TabHeaderRect { group, index: 1, bounds }],
            column_headers: vec![ColumnHeaderRect { side: DockSide::Right, index: 0, bounds }],
            splitters: vec![SplitterRect { kind: SplitterKind::Group, index: 0, bounds }],
            floating_titles: vec![FloatingTitleRect { floating, bounds }],
            floating_resize_edges: vec![FloatingResizeEdgeRect {
                floating,
                edge: ResizeEdge::BottomRight,
                bounds,
            }],
        };

        assert_eq!(rects.panels[0].panel, panel);
        assert_eq!(rects.tab_headers[0].index, 1);
        assert_eq!(rects.column_headers[0].side, DockSide::Right);
        assert_eq!(rects.splitters[0].kind, SplitterKind::Group);
        assert_eq!(rects.floating_titles[0].floating, floating);
        assert_eq!(rects.floating_resize_edges[0].edge, ResizeEdge::BottomRight);
        assert_eq!(rects.panels[0].bounds, bounds);
        assert_eq!(rects.canvas.size, Size::new(1280.0, 720.0));
    }
}
