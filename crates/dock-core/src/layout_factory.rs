use std::collections::HashSet;

use slotmap::SlotMap;

use crate::{
    CanvasState, ChromeMode, Column, ColumnMode, DockArea, DockAreaId, DockMetrics, DockSide,
    FloatingGroup, FloatingGroupId, Layout, Node, NodeId, Panel, PanelConstraints, PanelId,
    PanelInstance, PanelKey, Profile, Size, Tabs,
};

const REQUIRED_PANEL_KEYS: [&str; 4] = ["layers", "color", "properties", "history"];

/// Creates the required baseline composition without constructing any UI.
///
/// The application supplies its descriptor keys and constraints, keeping the
/// registry and PanelDescriptor type in app. The same composition is built for
/// the explicitly selected profile; profile-specific restrictions are enforced
/// by Layout validation.
pub fn create_initial_layout(
    profile: Profile,
    descriptors: &[(PanelKey, PanelConstraints)],
    metrics: DockMetrics,
) -> Result<Layout, &'static str> {
    metrics.validate().map_err(|_| "dock metrics are invalid")?;

    let mut seen = HashSet::with_capacity(descriptors.len());
    for (key, constraints) in descriptors {
        if !seen.insert(key.as_str()) {
            return Err("panel descriptor keys must be unique");
        }
        constraints
            .validate()
            .map_err(|_| "panel descriptor contains invalid constraints")?;
    }

    for key in REQUIRED_PANEL_KEYS {
        if !descriptors.iter().any(|(candidate, _)| candidate.as_str() == key) {
            return Err(match key {
                "layers" => "missing required panel descriptor: layers",
                "color" => "missing required panel descriptor: color",
                "properties" => "missing required panel descriptor: properties",
                _ => "missing required panel descriptor: history",
            });
        }
    }

    let descriptor = |key: &str| {
        descriptors
            .iter()
            .find(|(candidate, _)| candidate.as_str() == key)
            .map(|(key, constraints)| (key.clone(), *constraints))
    };

    let (layers_key, layers_constraints) = descriptor("layers").expect("required key checked");
    let (color_key, color_constraints) = descriptor("color").expect("required key checked");
    let (properties_key, properties_constraints) =
        descriptor("properties").expect("required key checked");
    let (history_key, history_constraints) =
        descriptor("history").expect("required key checked");

    let mut panel_instances = SlotMap::<PanelId, PanelInstance>::with_key();
    let layers_id = panel_instances.insert(PanelInstance::new(layers_key, layers_constraints));
    let color_id = panel_instances.insert(PanelInstance::new(color_key, color_constraints));
    let properties_id =
        panel_instances.insert(PanelInstance::new(properties_key, properties_constraints));
    let history_id = panel_instances.insert(PanelInstance::new(history_key, history_constraints));

    let mut nodes = SlotMap::<NodeId, Node>::with_key();
    let layers_node = nodes.insert(Node::Panel(Panel::new(layers_id)));
    let color_node = nodes.insert(Node::Panel(Panel::new(color_id)));
    let properties_node = nodes.insert(Node::Panel(Panel::new(properties_id)));
    let history_node = nodes.insert(Node::Panel(Panel::new(history_id)));

    let appearance_group = nodes.insert(Node::Tabs(Tabs {
        children: vec![layers_node, color_node],
        active_index: 0,
        collapsed: false,
    }));
    let information_group = nodes.insert(Node::Tabs(Tabs {
        children: vec![properties_node, history_node],
        active_index: 0,
        collapsed: false,
    }));

    let column_root = nodes.insert(Node::Column(Column {
        width: metrics.default_column_width,
        mode: ColumnMode::Expanded,
        groups: vec![appearance_group, information_group],
        group_ratios: vec![0.5, 0.5],
    }));

    let mut dock_areas = SlotMap::<DockAreaId, DockArea>::with_key();
    let right_area = dock_areas.insert(DockArea::main(DockSide::Right, column_root));

    let floating_groups = SlotMap::<FloatingGroupId, FloatingGroup>::with_key();

    Ok(Layout::from_parts(
        nodes,
        panel_instances,
        dock_areas,
        [None, Some(right_area)],
        floating_groups,
        CanvasState::default(),
        profile,
        0,
        ChromeMode::Visible,
    ))
}

/// A convenient baseline set of layout constraints for tests and examples.
///
/// Production app code should usually pass constraints from its registry.
pub fn baseline_panel_constraints() -> PanelConstraints {
    PanelConstraints::new(
        Size::new(120.0, 80.0),
        Size::new(280.0, 220.0),
        None,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::{baseline_panel_constraints, create_initial_layout};
    use crate::{Column, DockMetrics, DockSide, Node, PanelConstraints, PanelKey, Profile, Size};

    fn descriptors() -> Vec<(PanelKey, PanelConstraints)> {
        ["layers", "color", "properties", "history"]
            .into_iter()
            .map(|key| (PanelKey::new(key).unwrap(), baseline_panel_constraints()))
            .collect()
    }

    #[test]
    fn creates_the_required_right_column_and_two_tab_groups() {
        let layout =
            create_initial_layout(Profile::Photoshop, &descriptors(), DockMetrics::default())
                .unwrap();

        assert_eq!(layout.profile(), Profile::Photoshop);
        assert_eq!(layout.panel_instances().count(), 4);
        assert_eq!(layout.nodes().count(), 7);
        assert_eq!(layout.floating_groups().count(), 0);
        assert!(layout.main_area(DockSide::Left).is_none());

        let right = layout.main_area(DockSide::Right).unwrap();
        let Node::Column(column) = layout.node(right.root).unwrap() else {
            panic!("right main area must be rooted at a Column");
        };
        assert_eq!(column.width, 280.0);
        assert_eq!(column.group_ratios, vec![0.5, 0.5]);
        assert_eq!(column.groups.len(), 2);

        let Node::Tabs(first) = layout.node(column.groups[0]).unwrap() else {
            panic!("first group must be Tabs");
        };
        let first_keys = first
            .children
            .iter()
            .map(|id| match layout.node(*id).unwrap() {
                Node::Panel(panel) => layout.panel_instance(panel.panel_id).unwrap().key.as_str(),
                other => panic!("tab child must be Panel, got {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(first_keys, vec!["layers", "color"]);
        assert_eq!(first.active_index, 0);

        let Node::Tabs(second) = layout.node(column.groups[1]).unwrap() else {
            panic!("second group must be Tabs");
        };
        let second_keys = second
            .children
            .iter()
            .map(|id| match layout.node(*id).unwrap() {
                Node::Panel(panel) => layout.panel_instance(panel.panel_id).unwrap().key.as_str(),
                other => panic!("tab child must be Panel, got {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(second_keys, vec!["properties", "history"]);
        assert_eq!(second.active_index, 0);
    }

    #[test]
    fn factory_requires_all_builtin_keys_and_unique_descriptors() {
        let mut incomplete = descriptors();
        incomplete.retain(|(key, _)| key.as_str() != "history");
        assert_eq!(
            create_initial_layout(Profile::Generic, &incomplete, DockMetrics::default())
                .unwrap_err(),
            "missing required panel descriptor: history"
        );

        let mut duplicate = descriptors();
        duplicate.push(duplicate[0].clone());
        assert_eq!(
            create_initial_layout(Profile::Generic, &duplicate, DockMetrics::default())
                .unwrap_err(),
            "panel descriptor keys must be unique"
        );
    }

    #[test]
    fn factory_rejects_invalid_metrics_and_constraints() {
        let mut metrics = DockMetrics::default();
        metrics.default_column_width = f32::NAN;
        assert!(create_initial_layout(Profile::Generic, &descriptors(), metrics).is_err());

        let mut items = descriptors();
        items[0].1 = PanelConstraints::new(
            Size::new(400.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );
        assert!(create_initial_layout(Profile::Generic, &items, DockMetrics::default()).is_err());
    }

    #[test]
    fn factory_accepts_generic_profile_without_inferring_it_from_tree_shape() {
        let layout =
            create_initial_layout(Profile::Generic, &descriptors(), DockMetrics::default())
                .unwrap();
        assert_eq!(layout.profile(), Profile::Generic);
        assert!(matches!(
            layout.node(layout.main_area(DockSide::Right).unwrap().root),
            Some(Node::Column(Column { .. }))
        ));
    }
}
