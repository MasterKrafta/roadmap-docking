use crate::{
    ColumnRect, DropZoneRect, FloatingGroupRect, FloatingResizeEdgeRect, FlyoutRect, GroupRect,
    PanelRect, Rect, SplitterRect, TabHeaderRect, TabOverflowRect, TabStripRect, VerticalOverflow,
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
    /// Per-group overflow buttons and the panel tabs omitted from visible headers.
    #[cfg_attr(feature = "serde", serde(default))]
    pub tab_overflows: Vec<TabOverflowRect>,
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

/// Shared invariant checker for geometry snapshots produced by layout tests.
#[cfg(test)]
pub(crate) fn assert_layout_rects_invariants(rects: &LayoutRects) {
    let mut emitted = Vec::<(&'static str, Rect)>::new();
    emitted.extend(rects.viewport.map(|rect| ("viewport", rect)));
    emitted.extend(rects.canvas.map(|rect| ("canvas", rect)));
    emitted.extend(rects.columns.iter().map(|item| ("column", item.rect)));
    emitted.extend(rects.groups.iter().map(|item| ("group", item.rect)));
    emitted.extend(rects.tab_strips.iter().map(|item| ("tab strip", item.rect)));
    emitted.extend(rects.tab_headers.iter().map(|item| ("tab header", item.rect)));
    emitted.extend(
        rects
            .tab_overflows
            .iter()
            .map(|item| ("tab overflow button", item.button_rect)),
    );
    emitted.extend(rects.panels.iter().map(|item| ("panel", item.rect)));
    emitted.extend(rects.splitters.iter().map(|item| ("splitter", item.rect)));
    emitted.extend(
        rects
            .floating_groups
            .iter()
            .map(|item| ("floating group", item.rect)),
    );
    emitted.extend(
        rects
            .floating_resize_edges
            .iter()
            .map(|item| ("floating resize edge", item.rect)),
    );
    emitted.extend(rects.flyout.map(|item| ("flyout", item.rect)));
    emitted.extend(rects.drop_zones.iter().map(|item| ("drop zone", item.rect)));
    for overflow in &rects.vertical_overflow {
        emitted.push(("vertical overflow viewport", overflow.viewport));
        emitted.push(("vertical overflow content", overflow.content_extent));
    }

    for (kind, rect) in &emitted {
        assert!(
            rect.origin.is_finite(),
            "non-finite origin for emitted {kind}: {rect:?}"
        );
        assert!(
            rect.size.is_finite(),
            "non-finite size for emitted {kind}: {rect:?}"
        );
        assert!(
            rect.size.width >= 0.0 && rect.size.height >= 0.0,
            "negative dimension for emitted {kind}: {rect:?}"
        );
        assert_eq!(
            rect.validate(),
            Ok(()),
            "invalid or unrepresentable edge in emitted {kind}: {rect:?}"
        );

        let right = rect
            .checked_right()
            .expect("validated emitted rectangle must have a finite right edge");
        let bottom = rect
            .checked_bottom()
            .expect("validated emitted rectangle must have a finite bottom edge");
        assert!(right.is_finite(), "non-finite right edge for {kind}: {rect:?}");
        assert!(
            bottom.is_finite(),
            "non-finite bottom edge for {kind}: {rect:?}"
        );
        if rect.size.width > 0.0 {
            assert!(
                right > rect.origin.x,
                "positive width has no distinct right edge for {kind}: {rect:?}"
            );
        }
        if rect.size.height > 0.0 {
            assert!(
                bottom > rect.origin.y,
                "positive height has no distinct bottom edge for {kind}: {rect:?}"
            );
        }
    }

    // These rectangles are used directly as hit targets or resize/drop affordances.
    let mut interactive = Vec::<(&'static str, Rect)>::new();
    interactive.extend(
        rects
            .tab_headers
            .iter()
            .map(|item| ("tab header hit target", item.rect)),
    );
    interactive.extend(
        rects
            .tab_overflows
            .iter()
            .map(|item| ("tab overflow hit target", item.button_rect)),
    );
    interactive.extend(
        rects
            .splitters
            .iter()
            .map(|item| ("splitter hit target", item.rect)),
    );
    interactive.extend(
        rects
            .floating_resize_edges
            .iter()
            .map(|item| ("floating resize hit target", item.rect)),
    );
    interactive.extend(
        rects
            .drop_zones
            .iter()
            .map(|item| ("drop zone hit target", item.rect)),
    );

    for (kind, rect) in interactive {
        assert_eq!(
            rect.validate_non_empty(),
            Ok(()),
            "empty or invalid {kind}: {rect:?}"
        );
    }
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
        assert!(rects.tab_overflows.is_empty());
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
        assert_eq!(
            serde_json::from_str::<LayoutRects>(&encoded).unwrap(),
            rects
        );
    }
}
