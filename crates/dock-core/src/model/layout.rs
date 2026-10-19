use slotmap::SlotMap;

use crate::geometry::{DockMetrics, Size};
use crate::ids::{FloatingGroupId, NodeId, PanelId, PanelKey};

use super::{
    CanvasState, Column, ColumnMode, DockArea, DockSide, FloatingGroup, Node, PanelConstraints,
    PanelInstance, Profile, Tabs,
};

/// Complete dock layout model, independent of GPUI, windows, and panel content.
///
/// Equality is intentionally not derived because SlotMap does not implement PartialEq;
/// compare the public model projections or focused values in tests instead.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layout {
    nodes: SlotMap<NodeId, Node>,
    panels: SlotMap<PanelId, PanelInstance>,
    dock_areas: Vec<DockArea>,
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
}

impl Layout {
    pub fn new(profile: Profile, canvas: CanvasState) -> Self {
        Self {
            nodes: SlotMap::with_key(),
            panels: SlotMap::with_key(),
            dock_areas: Vec::new(),
            floating_groups: SlotMap::with_key(),
            canvas,
            profile,
            version: 0,
        }
    }

    /// Create the initial Photoshop composition without depending on GPUI.
    ///
    /// The four built-in panels each have one instance. The Canvas exists as a protected
    /// region in the model; its actual window-relative geometry is assigned by the layout
    /// geometry stage. Panel constraints are provisional until the app registry is added.
    pub fn initial_photoshop(metrics: &DockMetrics) -> Result<Self, &'static str> {
        metrics.validate()?;

        let constraints = PanelConstraints::new(
            Size::new(metrics.min_column_width, metrics.min_group_height),
            Size::new(metrics.default_column_width, metrics.min_group_height),
            Size::new(f32::MAX, f32::MAX),
            true,
        );

        let mut layout = Self::new(Profile::Photoshop, CanvasState::default());
        let layers = layout.insert_initial_panel("layers", constraints)?;
        let color = layout.insert_initial_panel("color", constraints)?;
        let properties = layout.insert_initial_panel("properties", constraints)?;
        let history = layout.insert_initial_panel("history", constraints)?;

        let layers_group = layout.insert_node(Node::Tabs(Tabs {
            panels: vec![layers, color],
            active_index: 0,
        }));
        let properties_group = layout.insert_node(Node::Tabs(Tabs {
            panels: vec![properties, history],
            active_index: 0,
        }));

        layout.push_dock_area(DockArea {
            side: DockSide::Right,
            columns: vec![Column {
                width: metrics.default_column_width,
                mode: ColumnMode::Expanded,
                groups: vec![layers_group, properties_group],
                group_ratios: vec![0.5, 0.5],
            }],
        });

        Ok(layout)
    }

    fn insert_initial_panel(
        &mut self,
        key: &str,
        constraints: PanelConstraints,
    ) -> Result<PanelId, &'static str> {
        let key = PanelKey::new(key)?;
        Ok(self.insert_panel(PanelInstance::new(key, constraints)))
    }

    pub fn nodes(&self) -> &SlotMap<NodeId, Node> { &self.nodes }
    pub fn panels(&self) -> &SlotMap<PanelId, PanelInstance> { &self.panels }
    pub fn dock_areas(&self) -> &[DockArea] { &self.dock_areas }
    pub fn floating_groups(&self) -> &SlotMap<FloatingGroupId, FloatingGroup> { &self.floating_groups }
    pub const fn canvas(&self) -> &CanvasState { &self.canvas }
    pub const fn profile(&self) -> Profile { self.profile }
    pub const fn version(&self) -> u64 { self.version }

    // Crate-internal construction/mutation hooks. Public callers can only read the model;
    // user-visible mutation will be routed through Layout::apply(DockCommand) in stage 3.
    pub(crate) fn insert_node(&mut self, node: Node) -> NodeId { self.nodes.insert(node) }
    pub(crate) fn insert_panel(&mut self, panel: PanelInstance) -> PanelId { self.panels.insert(panel) }
    pub(crate) fn push_dock_area(&mut self, area: DockArea) { self.dock_areas.push(area); }
    pub(crate) fn insert_floating_group(&mut self, group: FloatingGroup) -> FloatingGroupId {
        self.floating_groups.insert(group)
    }
    pub(crate) fn set_canvas(&mut self, canvas: CanvasState) { self.canvas = canvas; }
    pub(crate) fn panel_mut(&mut self, id: PanelId) -> Option<&mut PanelInstance> { self.panels.get_mut(id) }

    pub(crate) fn advance_version(&mut self) -> Result<u64, &'static str> {
        let next = self.version.checked_add(1).ok_or("layout version exhausted")?;
        self.version = next;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;
    use crate::{
        CanvasState, ColumnMode, DockMetrics, DockSide, Point, Profile, Rect, Size,
    };

    fn metrics() -> DockMetrics {
        DockMetrics {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        }
    }

    fn empty_layout() -> Layout {
        Layout::new(Profile::Photoshop, CanvasState {
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(1200.0, 800.0)),
        })
    }

    #[test]
    fn starts_with_empty_arenas_and_version_zero() {
        let layout = empty_layout();
        assert!(layout.nodes().is_empty());
        assert!(layout.panels().is_empty());
        assert!(layout.dock_areas().is_empty());
        assert!(layout.floating_groups().is_empty());
        assert_eq!(layout.profile(), Profile::Photoshop);
        assert_eq!(layout.version(), 0);
    }

    #[test]
    fn version_advances_only_when_a_change_is_recorded() {
        let mut layout = empty_layout();
        assert_eq!(layout.advance_version(), Ok(1));
        assert_eq!(layout.advance_version(), Ok(2));
        assert_eq!(layout.version(), 2);
    }

    #[test]
    fn version_overflow_is_reported_without_wrapping() {
        let mut layout = empty_layout();
        layout.version = u64::MAX;
        assert!(layout.advance_version().is_err());
        assert_eq!(layout.version(), u64::MAX);
    }

    #[test]
    fn initial_photoshop_layout_has_canvas_one_right_column_and_two_groups() {
        let layout = Layout::initial_photoshop(&metrics()).expect("valid metrics");

        assert_eq!(layout.profile(), Profile::Photoshop);
        assert_eq!(layout.version(), 0);
        assert!(layout.floating_groups().is_empty());
        assert_eq!(layout.dock_areas().len(), 1);

        let area = &layout.dock_areas()[0];
        assert_eq!(area.side, DockSide::Right);
        assert_eq!(area.columns.len(), 1);

        let column = &area.columns[0];
        assert!((column.width - 280.0).abs() < 1e-3);
        assert_eq!(column.mode, ColumnMode::Expanded);
        assert_eq!(column.groups.len(), 2);
        assert_eq!(column.group_ratios, vec![0.5, 0.5]);

        let layers_tabs = match layout.nodes().get(column.groups[0]) {
            Some(crate::Node::Tabs(tabs)) => tabs,
            other => panic!("expected first column group to be Tabs, got {other:?}"),
        };
        let properties_tabs = match layout.nodes().get(column.groups[1]) {
            Some(crate::Node::Tabs(tabs)) => tabs,
            other => panic!("expected second column group to be Tabs, got {other:?}"),
        };

        assert_eq!(layers_tabs.panels.len(), 2);
        assert_eq!(layers_tabs.active_index, 0);
        assert_eq!(layout.panels().get(layers_tabs.panels[0]).map(|p| p.key().as_str()), Some("layers"));
        assert_eq!(layout.panels().get(layers_tabs.panels[1]).map(|p| p.key().as_str()), Some("color"));

        assert_eq!(properties_tabs.panels.len(), 2);
        assert_eq!(properties_tabs.active_index, 0);
        assert_eq!(layout.panels().get(properties_tabs.panels[0]).map(|p| p.key().as_str()), Some("properties"));
        assert_eq!(layout.panels().get(properties_tabs.panels[1]).map(|p| p.key().as_str()), Some("history"));

        assert_eq!(layout.panels().len(), 4);
        assert!(layout.panels().values().all(|panel| panel.constraints().is_singleton()));
        // Canvas is always present independently of the dock tree.
        let _canvas = layout.canvas();
    }

    #[test]
    fn initial_layout_rejects_invalid_metrics() {
        let mut invalid = metrics();
        invalid.default_column_width = f32::NAN;
        assert!(Layout::initial_photoshop(&invalid).is_err());
    }
}
