use crate::{
    ColumnRect, DropZoneRect, FloatingGroupRect, FloatingResizeEdgeRect, GroupRect,
    PanelRect, SplitterRect, TabHeaderRect, TabStripRect,
};

/// Complete pure-data geometry snapshot produced by the geometry pass.
///
/// Collections retain semantic identifiers so hit-testing and drop resolution
/// can map rectangles back to model commands. No UI toolkit objects or
/// coordinate-space handles are stored here.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LayoutRects {
    pub columns: Vec<ColumnRect>,
    pub groups: Vec<GroupRect>,
    pub tab_strips: Vec<TabStripRect>,
    pub tab_headers: Vec<TabHeaderRect>,
    pub panels: Vec<PanelRect>,
    pub splitters: Vec<SplitterRect>,
    pub floating_groups: Vec<FloatingGroupRect>,
    pub floating_resize_edges: Vec<FloatingResizeEdgeRect>,
    pub drop_zones: Vec<DropZoneRect>,
}

#[cfg(test)]
mod tests {
    use super::LayoutRects;

    #[test]
    fn default_geometry_snapshot_has_empty_collections() {
        let rects = LayoutRects::default();
        assert!(rects.columns.is_empty());
        assert!(rects.groups.is_empty());
        assert!(rects.tab_strips.is_empty());
        assert!(rects.tab_headers.is_empty());
        assert!(rects.panels.is_empty());
        assert!(rects.splitters.is_empty());
        assert!(rects.floating_groups.is_empty());
        assert!(rects.floating_resize_edges.is_empty());
        assert!(rects.drop_zones.is_empty());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn empty_snapshot_round_trips_through_serde() {
        let rects = LayoutRects::default();
        let encoded = serde_json::to_string(&rects).unwrap();
        assert_eq!(serde_json::from_str::<LayoutRects>(&encoded).unwrap(), rects);
    }
}
