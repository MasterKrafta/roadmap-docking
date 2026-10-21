//! Test-only builder for assembling docking layouts through the Rust API.
//!
//! Keeping this helper separate from production modules ensures tests exercise
//! actual runtime handles and tree nodes without inventing a text schema/parser.

use slotmap::SlotMap;

use crate::{
    CanvasState, ChromeMode, Column, ColumnMode, DockArea, DockAreaId, DockSide, FloatingGroup,
    FloatingGroupId, Layout, Node, NodeId, Panel, PanelConstraints, PanelId, PanelInstance, PanelKey,
    Profile, Rect, Split, Tabs,
};

pub(crate) struct LayoutBuilder {
    nodes: SlotMap<NodeId, Node>,
    panel_instances: SlotMap<PanelId, PanelInstance>,
    dock_areas: SlotMap<DockAreaId, DockArea>,
    main_areas: [Option<DockAreaId>; 2],
    floating_groups: SlotMap<FloatingGroupId, FloatingGroup>,
    canvas: CanvasState,
    profile: Profile,
    version: u64,
    chrome: ChromeMode,
}

impl LayoutBuilder {
    pub(crate) fn new(profile: Profile) -> Self {
        Self {
            nodes: SlotMap::with_key(),
            panel_instances: SlotMap::with_key(),
            dock_areas: SlotMap::with_key(),
            main_areas: [None, None],
            floating_groups: SlotMap::with_key(),
            canvas: CanvasState::default(),
            profile,
            version: 0,
            chrome: ChromeMode::Visible,
        }
    }

    pub(crate) fn panel_instance(
        &mut self,
        key: PanelKey,
        constraints: PanelConstraints,
    ) -> PanelId {
        self.panel_instances
            .insert(PanelInstance::new(key, constraints))
    }

    pub(crate) fn panel(&mut self, key: PanelKey, constraints: PanelConstraints) -> (PanelId, NodeId) {
        let panel_id = self.panel_instance(key, constraints);
        let node_id = self.panel_node(panel_id);
        (panel_id, node_id)
    }

    /// Creates another leaf for an existing PanelId; useful for invalid-state tests.
    pub(crate) fn panel_node(&mut self, panel_id: PanelId) -> NodeId {
        self.nodes.insert(Node::Panel(Panel::new(panel_id)))
    }

    pub(crate) fn node(&mut self, node: Node) -> NodeId {
        self.nodes.insert(node)
    }

    pub(crate) fn tabs(
        &mut self,
        children: Vec<NodeId>,
        active_index: usize,
        collapsed: bool,
    ) -> NodeId {
        self.node(Node::Tabs(Tabs {
            children,
            active_index,
            collapsed,
        }))
    }

    pub(crate) fn split(
        &mut self,
        axis: crate::Axis,
        children: Vec<NodeId>,
        ratios: Vec<f32>,
    ) -> NodeId {
        self.node(Node::Split(Split {
            axis,
            children,
            ratios,
        }))
    }

    pub(crate) fn column(
        &mut self,
        width: f32,
        mode: ColumnMode,
        groups: Vec<NodeId>,
        group_ratios: Vec<f32>,
    ) -> NodeId {
        self.node(Node::Column(Column {
            width,
            mode,
            groups,
            group_ratios,
        }))
    }

    pub(crate) fn main_area(&mut self, side: DockSide, root: NodeId) -> DockAreaId {
        let id = self.dock_areas.insert(DockArea::main(side, root));
        let index = match side {
            DockSide::Left => 0,
            DockSide::Right => 1,
        };
        self.main_areas[index] = Some(id);
        id
    }

    pub(crate) fn floating_group(
        &mut self,
        root: NodeId,
        bounds: Rect,
        z_order: i32,
    ) -> FloatingGroupId {
        let area = self.dock_areas.insert(DockArea::floating(root));
        self.floating_groups
            .insert(FloatingGroup::new(area, bounds, z_order))
    }

    pub(crate) fn canvas(&mut self, canvas: CanvasState) {
        self.canvas = canvas;
    }

    pub(crate) fn set_version(&mut self, version: u64) {
        self.version = version;
    }

    pub(crate) fn remove_node(&mut self, id: NodeId) -> Option<Node> {
        self.nodes.remove(id)
    }

    pub(crate) fn remove_panel_instance(&mut self, id: PanelId) -> Option<PanelInstance> {
        self.panel_instances.remove(id)
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    pub(crate) fn build(self) -> Layout {
        Layout::from_parts(
            self.nodes,
            self.panel_instances,
            self.dock_areas,
            self.main_areas,
            self.floating_groups,
            self.canvas,
            self.profile,
            self.version,
            self.chrome,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::LayoutBuilder;
    use crate::{
        Axis, CanvasState, ColumnMode, DockMetrics, DockSide, GeometryError, Node, NodeId,
        PanelConstraints, PanelId, PanelKey, Point, Profile, Rect, Size, Split, Tabs,
        ValidationError,
    };
    use proptest::prelude::*;

    fn constraints(singleton: bool) -> PanelConstraints {
        PanelConstraints::new(
            Size::new(80.0, 60.0),
            Size::new(180.0, 140.0),
            Some(Size::new(800.0, 600.0)),
            singleton,
        )
    }

    fn key(value: impl Into<String>) -> PanelKey {
        PanelKey::new(value).expect("test panel key must be valid")
    }

    fn generated_layout(group_sizes: &[usize], ratios: Vec<f32>) -> crate::Layout {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let mut groups = Vec::with_capacity(group_sizes.len());
        let total_panels: usize = group_sizes.iter().sum();
        let mut panel_index = 0usize;

        for group_size in group_sizes {
            let mut panel_nodes = Vec::with_capacity(*group_size);
            for _ in 0..*group_size {
                let (_, panel_node) = builder.panel(
                    key(format!("prop-panel-{panel_index}")),
                    constraints(false),
                );
                panel_index += 1;
                panel_nodes.push(panel_node);
            }
            let group = builder.tabs(panel_nodes, 0, false);
            groups.push(group);
        }

        debug_assert_eq!(panel_index, total_panels);
        let equal_ratios = if groups.is_empty() {
            Vec::new()
        } else {
            vec![1.0 / groups.len() as f32; groups.len()]
        };
        let chosen_ratios = if ratios.is_empty() { equal_ratios } else { ratios };
        let root = builder.column(
            DockMetrics::default().default_column_width,
            ColumnMode::Expanded,
            groups,
            chosen_ratios,
        );
        builder.main_area(DockSide::Right, root);
        builder.build()
    }

    fn snapshot(
        layout: &crate::Layout,
    ) -> (
        Vec<(NodeId, String)>,
        Vec<(PanelId, String)>,
        Option<NodeId>,
        usize,
        usize,
    ) {
        (
            layout
                .nodes()
                .map(|(id, node)| (id, format!("{node:?}")))
                .collect(),
            layout
                .panel_instances()
                .map(|(id, panel)| (id, format!("{panel:?}")))
                .collect(),
            layout.main_area(DockSide::Right).map(|area| area.root),
            layout.dock_areas().count(),
            layout.floating_groups().count(),
        )
    }

    #[test]
    fn builder_creates_layout_using_rust_api_without_a_schema_parser() {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel) = builder.panel(key("layers"), constraints(false));
        let tabs = builder.tabs(vec![panel], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![tabs], vec![1.0]);
        builder.main_area(DockSide::Right, column);
        let layout = builder.build();

        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
        assert_eq!(layout.panel_instances().count(), 1);
        assert_eq!(layout.nodes().count(), 3);
    }

    #[test]
    fn stale_deleted_node_reference_is_reported() {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel) = builder.panel(key("layers"), constraints(false));
        let tabs = builder.tabs(vec![panel], 0, false);
        let root = builder.column(280.0, ColumnMode::Expanded, vec![tabs], vec![1.0]);
        builder.main_area(DockSide::Right, root);
        assert!(builder.remove_node(panel).is_some());

        let layout = builder.build();
        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::MissingNode { missing, .. } if *missing == panel
        )));
    }

    #[test]
    fn detects_the_same_panel_instance_in_main_and_floating_areas() {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let panel_id = builder.panel_instance(key("layers"), constraints(false));
        let main_leaf = builder.panel_node(panel_id);
        let main_tabs = builder.tabs(vec![main_leaf], 0, false);
        let main_column =
            builder.column(280.0, ColumnMode::Expanded, vec![main_tabs], vec![1.0]);
        builder.main_area(DockSide::Right, main_column);

        let floating_leaf = builder.panel_node(panel_id);
        let floating_tabs = builder.tabs(vec![floating_leaf], 0, false);
        builder.floating_group(
            floating_tabs,
            Rect::new(Point::new(60.0, 80.0), Size::new(420.0, 300.0)),
            1,
        );

        let layout = builder.build();
        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::DuplicatePanelPlacement { panel, .. } if *panel == panel_id
        )));
    }

    #[test]
    fn detects_duplicate_singleton_instances_with_same_key() {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, first_leaf) = builder.panel(key("layers"), constraints(true));
        let (_, second_leaf) = builder.panel(key("layers"), constraints(true));
        let first_tabs = builder.tabs(vec![first_leaf], 0, false);
        let second_tabs = builder.tabs(vec![second_leaf], 0, false);
        let root = builder.column(
            280.0,
            ColumnMode::Expanded,
            vec![first_tabs, second_tabs],
            vec![0.5, 0.5],
        );
        builder.main_area(DockSide::Right, root);

        let layout = builder.build();
        assert!(layout.validate(&DockMetrics::default()).unwrap_err().iter().any(
            |error| matches!(error, ValidationError::DuplicateSingletonPanel { .. })
        ));
    }

    #[test]
    fn detects_cycle_and_unreachable_node() {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let (_, panel) = builder.panel(key("layers"), constraints(false));
        let tabs = builder.tabs(vec![panel], 0, false);
        let root = builder.column(280.0, ColumnMode::Expanded, vec![tabs], vec![1.0]);
        builder.main_area(DockSide::Right, root);

        let (_, orphan_leaf) = builder.panel(key("orphan-panel"), constraints(false));
        let orphan_cycle = builder.tabs(Vec::new(), 0, false);
        let Node::Tabs(cycle_tabs) = builder.node_mut(orphan_cycle).unwrap() else {
            unreachable!();
        };
        cycle_tabs.children.push(orphan_cycle);

        let layout = builder.build();
        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(error, ValidationError::NodeCycle { .. })));
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::UnreachableNode { node } if *node == orphan_leaf
        )));
    }

    #[test]
    fn canvas_is_mandatory_and_is_not_representable_as_a_tree_node() {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        builder.canvas(CanvasState::new(Some(Rect::new(
            Point::new(200.0, 0.0),
            Size::new(900.0, 700.0),
        ))));
        let (_, panel) = builder.panel(key("layers"), constraints(false));
        let tabs = builder.tabs(vec![panel], 0, false);
        let root = builder.column(280.0, ColumnMode::Expanded, vec![tabs], vec![1.0]);
        builder.main_area(DockSide::Right, root);
        let layout = builder.build();

        assert_eq!(layout.canvas().last_known_bounds(), Some(Rect::new(
            Point::new(200.0, 0.0),
            Size::new(900.0, 700.0),
        )));
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
        // Node has only Split/Tabs/Panel/Column variants. Canvas remains an
        // independent Layout field and cannot accidentally enter the tree.
        assert!(layout.nodes().all(|(_, node)| matches!(
            node,
            Node::Split(_) | Node::Tabs(_) | Node::Panel(_) | Node::Column(_)
        )));
    }

    #[test]
    fn rejects_too_many_photoshop_main_columns() {
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let mut columns = Vec::new();
        for index in 0..5 {
            let (_, panel) = builder.panel(key(format!("column-panel-{index}")), constraints(false));
            let tabs = builder.tabs(vec![panel], 0, false);
            let column = builder.column(280.0, ColumnMode::Expanded, vec![tabs], vec![1.0]);
            columns.push(column);
        }
        let root = builder.split(Axis::X, columns, vec![0.2; 5]);
        builder.main_area(DockSide::Right, root);

        let layout = builder.build();
        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::TooManyColumns { actual: 5, max: 4 }
        )));
    }

    #[test]
    fn column_with_one_group_is_valid_and_kept_as_column() {
        let mut builder = LayoutBuilder::new(Profile::Photoshop);
        let (_, panel) = builder.panel(key("layers"), constraints(false));
        let tabs = builder.tabs(vec![panel], 0, false);
        let column = builder.column(280.0, ColumnMode::Expanded, vec![tabs], vec![1.0]);
        builder.main_area(DockSide::Right, column);
        let mut layout = builder.build();

        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
        layout.normalize(&DockMetrics::default()).unwrap();
        let root = layout.main_area(DockSide::Right).unwrap().root;
        assert!(matches!(layout.node(root), Some(Node::Column(column)) if column.groups.len() == 1));
    }

    #[test]
    fn empty_group_and_column_are_reported_then_removed_by_normalize() {
        let mut builder = LayoutBuilder::new(Profile::Generic);
        let empty_tabs = builder.tabs(Vec::new(), 0, false);
        let empty_column =
            builder.column(280.0, ColumnMode::Expanded, vec![empty_tabs], vec![1.0]);
        builder.main_area(DockSide::Right, empty_column);
        let mut layout = builder.build();

        let errors = layout.validate(&DockMetrics::default()).unwrap_err();
        assert!(errors.iter().any(|error| matches!(error, ValidationError::InvalidTabs { .. })));
        layout.normalize(&DockMetrics::default()).unwrap();
        assert_eq!(layout.nodes().count(), 0);
        assert!(layout.main_area(DockSide::Right).is_none());
        assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
    }

    #[test]
    fn normalize_is_idempotent_after_invalid_ratios_are_repaired() {
        let mut layout = generated_layout(&[2, 1, 3], vec![0.1, 0.1, 0.1, 0.7]);
        assert!(layout.validate(&DockMetrics::default()).is_err());
        layout.normalize(&DockMetrics::default()).unwrap();
        let after_first = snapshot(&layout);
        layout.normalize(&DockMetrics::default()).unwrap();
        assert_eq!(snapshot(&layout), after_first);
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]

        #[test]
        fn generated_nonempty_layouts_pass_validation(
            group_sizes in prop::collection::vec(1usize..5, 1..6)
        ) {
            let ratios = vec![1.0 / group_sizes.len() as f32; group_sizes.len()];
            let layout = generated_layout(&group_sizes, ratios);
            prop_assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
        }

        #[test]
        fn generated_ratio_repair_sequences_remain_valid_and_idempotent(
            group_sizes in prop::collection::vec(1usize..5, 1..6),
            ratios in prop::collection::vec(0.0f32..4.0, 0..8),
            normalize_passes in 1u8..6
        ) {
            let mut layout = generated_layout(&group_sizes, ratios);
            for _ in 0..normalize_passes {
                prop_assert_eq!(layout.normalize(&DockMetrics::default()), Ok(()));
                prop_assert_eq!(layout.validate(&DockMetrics::default()), Ok(()));
            }
            let normalized = snapshot(&layout);
            layout.normalize(&DockMetrics::default()).unwrap();
            prop_assert_eq!(snapshot(&layout), normalized);
        }
    }
}
