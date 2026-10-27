use std::collections::{HashMap, HashSet};

use dock_core::{
    ComputeRectsError, DockAreaId, DockMetrics, DropSource, DropZoneRect, DropZoneResolver, Layout,
    LayoutRects, Node, NodeId, PanelId, Rect,
};

use crate::chrome_geometry::normalize_docked_group_geometry;
use crate::tree_query::subtree_nodes;

/// A single immutable, normalized geometry snapshot for one render frame.
///
/// Geometry consumers should share this value so rendering and drop-zone
/// calculation use the same rectangles. The internal caches avoid repeating
/// tree walks when drawing separate dock areas.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameGeometry {
    layout_version: u64,
    rects: LayoutRects,
    nodes_by_area: HashMap<DockAreaId, HashSet<NodeId>>,
    nodes_by_root: HashMap<NodeId, HashSet<NodeId>>,
    rail_groups: HashSet<NodeId>,
    panel_nodes: HashMap<PanelId, NodeId>,
}

impl FrameGeometry {
    /// Computes and normalizes geometry once, then builds the per-frame caches.
    pub fn compute(
        layout: &Layout,
        viewport: Rect,
        metrics: &DockMetrics,
    ) -> Result<Self, ComputeRectsError> {
        let layout_version = layout.version();
        let mut rects = layout.compute_rects(viewport, metrics)?;
        normalize_docked_group_geometry(layout, metrics, &mut rects);

        let mut nodes_by_area = HashMap::new();
        let mut nodes_by_root = HashMap::<NodeId, HashSet<NodeId>>::new();
        for (area_id, area) in layout.dock_areas() {
            let nodes = nodes_by_root
                .entry(area.root)
                .or_insert_with(|| subtree_nodes(layout, area.root))
                .clone();
            nodes_by_area.insert(area_id, nodes);
        }
        // Scroll columns are subtrees inside a main area rather than independent
        // DockArea entries. Cache their roots too so rendering does not walk them
        // repeatedly while building the scroll viewport and its content.
        for column in &rects.columns {
            nodes_by_root
                .entry(column.column)
                .or_insert_with(|| subtree_nodes(layout, column.column));
        }

        let mut rail_groups = HashSet::new();
        for column_rect in &rects.columns {
            if matches!(
                layout.node(column_rect.column),
                Some(Node::Column(column)) if column.mode.is_rail()
            ) && let Ok(items) = layout.column_display_order(column_rect.column)
            {
                rail_groups.extend(items.into_iter().map(|item| item.group));
            }
        }

        let panel_nodes = layout
            .nodes()
            .filter_map(|(node_id, node)| match node {
                Node::Panel(panel) => Some((panel.panel_id, node_id)),
                _ => None,
            })
            .collect();

        Ok(Self {
            layout_version,
            rects,
            nodes_by_area,
            nodes_by_root,
            rail_groups,
            panel_nodes,
        })
    }

    /// Returns the layout revision used to compute this frame snapshot.
    pub const fn layout_version(&self) -> u64 {
        self.layout_version
    }

    /// Returns the normalized rectangle snapshot. It cannot be mutated here.
    pub fn rects(&self) -> &LayoutRects {
        &self.rects
    }

    /// Returns the cached set of tree nodes belonging to a dock area.
    pub fn nodes_in_area(&self, area: DockAreaId) -> Option<&HashSet<NodeId>> {
        self.nodes_by_area.get(&area)
    }

    /// Returns the cached set of tree nodes below an arbitrary dock-area or
    /// column root used during this frame.
    pub(crate) fn nodes_in_root(&self, root: NodeId) -> Option<&HashSet<NodeId>> {
        self.nodes_by_root.get(&root)
    }

    /// Returns the groups presented as rail entries in this frame.
    pub fn rail_groups(&self) -> &HashSet<NodeId> {
        &self.rail_groups
    }

    /// Looks up the tree node that owns a panel instance.
    pub fn panel_node(&self, panel: PanelId) -> Option<NodeId> {
        self.panel_nodes.get(&panel).copied()
    }

    /// Computes legal drop zones using this frame's normalized geometry.
    ///
    /// Core's API accepts a semantic `DropSource` and does not need the
    /// current pointer position; hit selection is handled by the drag session.
    pub fn drop_zones(
        &self,
        layout: &Layout,
        source: DropSource,
        metrics: &DockMetrics,
    ) -> Result<Vec<DropZoneRect>, ComputeRectsError> {
        layout.compute_drop_zones(source, &self.rects, metrics)
    }

    /// Prepares an owned resolver from this frame's normalized geometry snapshot.
    ///
    /// The returned resolver can be stored in drag-session state across GPUI callbacks.
    /// Callers must discard it if the layout, frame geometry, or metrics change.
    pub fn prepare_drop_resolver(
        &self,
        layout: &Layout,
        source: DropSource,
        metrics: &DockMetrics,
    ) -> Result<DropZoneResolver, ComputeRectsError> {
        layout.prepare_drop_resolver(source, &self.rects, metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::FrameGeometry;
    use dock_core::{
        create_initial_layout, DockMetrics, DockSide, PanelConstraints, PanelKey, Point, Profile,
        Rect, Size,
    };

    #[test]
    fn compute_is_deterministic_for_unchanged_layout() {
        let constraints = PanelConstraints::new(
            Size::new(80.0, 80.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            false,
        );
        let descriptors = ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), constraints))
            .collect::<Vec<_>>();
        let metrics = DockMetrics {
            header_height: 32.0,
            tab_strip_height: 24.0,
            ..DockMetrics::default()
        };
        let layout =
            create_initial_layout(Profile::Photoshop, &descriptors, metrics).unwrap();
        let viewport = Rect::new(Point::ZERO, Size::new(1000.0, 700.0));

        let mut normalized_once = layout.compute_rects(viewport, &metrics).unwrap();
        super::normalize_docked_group_geometry(&layout, &metrics, &mut normalized_once);

        let first = FrameGeometry::compute(&layout, viewport, &metrics).unwrap();
        let second = FrameGeometry::compute(&layout, viewport, &metrics).unwrap();

        assert_eq!(
            first.rects(),
            &normalized_once,
            "FrameGeometry should normalize a fresh Core snapshot exactly once"
        );
        assert_eq!(first, second);
        assert_eq!(first.rects().viewport, Some(viewport));
        let right_area = layout
            .dock_areas()
            .find_map(|(id, area)| (area.side == Some(DockSide::Right)).then_some(id))
            .expect("initial layout should contain a right main area");
        assert!(
            first.nodes_in_area(right_area).is_some(),
            "main area nodes should be cached"
        );
    }
}
