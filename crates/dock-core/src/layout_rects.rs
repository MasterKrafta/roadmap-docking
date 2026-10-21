use crate::{
    ColumnRect, DropZoneRect, FloatingGroupRect, FloatingResizeEdgeRect, FlyoutRect, GroupRect,
    PanelRect, Rect, SplitterRect, TabHeaderRect, TabStripRect, VerticalOverflow,
};

/// Complete pure-data geometry snapshot produced by the geometry pass.
///
/// Collections retain semantic identifiers so hit-testing and drop resolution
/// can map rectangles back to model commands. No UI toolkit objects or
/// coordinate-space handles are stored here.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayoutRects {
    /// Full logical-pixel viewport supplied to compute_rects.
    /// `None` means this snapshot has not been computed yet.
    #[cfg_attr(feature = "serde", serde(default))]
    pub viewport: Option<Rect>,
    /// Canvas rectangle left after main docks consume their logical widths.
    /// `None` means this snapshot has not been computed yet.
    pub canvas: Option<Rect>,
    pub columns: Vec<ColumnRect>,
    pub groups: Vec<GroupRect>,
    pub tab_strips: Vec<TabStripRect>,
    pub tab_headers: Vec<TabHeaderRect>,
    pub panels: Vec<PanelRect>,
    pub splitters: Vec<SplitterRect>,
    pub floating_groups: Vec<FloatingGroupRect>,
    pub floating_resize_edges: Vec<FloatingResizeEdgeRect>,
    /// Optional geometry for the active rail flyout, drawn above the Canvas.
    #[cfg_attr(feature = "serde", serde(default))]
    pub flyout: Option<FlyoutRect>,
    pub drop_zones: Vec<DropZoneRect>,
    /// Vertical stacks that exceed their allocated viewport at minimum sizes.
    pub vertical_overflow: Vec<VerticalOverflow>,
}

#[cfg(test)]
mod tests {
    use super::LayoutRects;

    #[test]
    fn default_geometry_snapshot_has_empty_collections() {
        let rects = LayoutRects::default();
        assert!(rects.viewport.is_none());
        assert!(rects.canvas.is_none());
        assert!(rects.columns.is_empty());
        assert!(rects.groups.is_empty());
        assert!(rects.tab_strips.is_empty());
        assert!(rects.tab_headers.is_empty());
        assert!(rects.panels.is_empty());
        assert!(rects.splitters.is_empty());
        assert!(rects.floating_groups.is_empty());
        assert!(rects.floating_resize_edges.is_empty());
        assert!(rects.flyout.is_none());
        assert!(rects.drop_zones.is_empty());
        assert!(rects.vertical_overflow.is_empty());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn empty_snapshot_round_trips_through_serde() {
        let rects = LayoutRects::default();
        let encoded = serde_json::to_string(&rects).unwrap();
        assert_eq!(serde_json::from_str::<LayoutRects>(&encoded).unwrap(), rects);
    }
}
