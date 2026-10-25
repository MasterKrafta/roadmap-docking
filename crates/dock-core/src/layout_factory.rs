use std::collections::HashSet;

use slotmap::SlotMap;

use crate::{
    CanvasState, ChromeMode, Column, ColumnMode, DockArea, DockAreaId, DockMetrics, DockSide,
    FloatingGroup, FloatingGroupId, Layout, Node, NodeId, Panel, PanelConstraints, PanelId,
    PanelInstance, PanelKey, Profile, Tabs,
};

const REQUIRED_PANEL_KEYS: [&str; 4] = ["layers", "color", "properties", "history"];

/// Typed reasons why the baseline layout factory could not create a layout.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InitialLayoutError {
    /// The supplied docking metrics are invalid.
    #[error("dock metrics are invalid: {reason}")]
    InvalidMetrics { reason: &'static str },
    /// Two panel descriptors use the same stable key.
    #[error("panel descriptor keys must be unique")]
    DuplicatePanelKey,
    /// A descriptor's size constraints are invalid.
    #[error("panel descriptor contains invalid constraints for '{key}': {source}")]
    InvalidPanelConstraints {
        key: String,
        #[source]
        source: crate::GeometryError,
    },
    /// One of the baseline panel descriptors is absent.
    #[error("missing required panel descriptor: {key}")]
    MissingRequiredPanel { key: &'static str },
    /// Group members have incompatible minimum/maximum width constraints.
    #[error("panel minimum width exceeds the maximum width allowed by its tab group")]
    ConflictingPanelWidthConstraints,
}

/// Creates the required baseline composition without constructing any UI.
///
/// Errors preserve their category and relevant validation details. The
/// application supplies descriptor keys and constraints, keeping the registry
/// and PanelDescriptor type in the UI adapter.
pub fn create_initial_layout(
    profile: Profile,
    descriptors: &[(PanelKey, PanelConstraints)],
    metrics: DockMetrics,
) -> Result<Layout, InitialLayoutError> {
    metrics
        .validate()
        .map_err(|reason| InitialLayoutError::InvalidMetrics { reason })?;

    let mut seen = HashSet::with_capacity(descriptors.len());
    for (key, constraints) in descriptors {
        if !seen.insert(key.as_str()) {
            return Err(InitialLayoutError::DuplicatePanelKey);
        }
        constraints.validate().map_err(|source| {
            InitialLayoutError::InvalidPanelConstraints {
                key: key.to_string(),
                source,
            }
        })?;
    }

    for key in REQUIRED_PANEL_KEYS {
        if !descriptors
            .iter()
            .any(|(candidate, _)| candidate.as_str() == key)
        {
            return Err(InitialLayoutError::MissingRequiredPanel { key });
        }
    }

    let descriptor = |key: &str| {
        descriptors
            .iter()
            .find(|(candidate, _)| candidate.as_str() == key)
            .map(|(key, constraints)| (key.clone(), *constraints))
    };

    let (layers_key, layers_constraints) = descriptor("layers")
        .ok_or(InitialLayoutError::MissingRequiredPanel { key: "layers" })?;
    let (color_key, color_constraints) = descriptor("color")
        .ok_or(InitialLayoutError::MissingRequiredPanel { key: "color" })?;
    let (properties_key, properties_constraints) = descriptor("properties")
        .ok_or(InitialLayoutError::MissingRequiredPanel { key: "properties" })?;
    let (history_key, history_constraints) = descriptor("history")
        .ok_or(InitialLayoutError::MissingRequiredPanel { key: "history" })?;

    // Both built-in groups are tab stacks, so their minimum width is the widest
    // panel minimum in that group. The column must satisfy both groups and the
    // configured minimum column metric.
    let minimum_column_width = metrics.min_column_width.max(
        layers_constraints
            .min_size
            .width
            .max(color_constraints.min_size.width)
            .max(properties_constraints.min_size.width)
            .max(history_constraints.min_size.width),
    );
    let tab_group_maximum = |left: PanelConstraints, right: PanelConstraints| {
        match (left.max_size, right.max_size) {
            (Some(left), Some(right)) => Some(left.width.min(right.width)),
            (Some(left), None) => Some(left.width),
            (None, Some(right)) => Some(right.width),
            (None, None) => None,
        }
    };
    let mut maximum_column_width: Option<f32> = None;
    for group_maximum in [
        tab_group_maximum(layers_constraints, color_constraints),
        tab_group_maximum(properties_constraints, history_constraints),
    ]
    .into_iter()
    .flatten()
    {
        maximum_column_width = Some(
            maximum_column_width.map_or(group_maximum, |current| current.min(group_maximum)),
        );
    }
    if maximum_column_width.is_some_and(|maximum| minimum_column_width > maximum) {
        return Err(InitialLayoutError::ConflictingPanelWidthConstraints);
    }
    let column_width = metrics
        .default_column_width
        .max(minimum_column_width)
        .min(maximum_column_width.unwrap_or(f32::MAX));

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
        width: column_width,
        mode: ColumnMode::Expanded,
        groups: vec![appearance_group, information_group],
        group_ratios: vec![0.5, 0.5],
        last_expanded_width: Some(column_width),
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

#[cfg(test)]
mod tests {
    use super::{create_initial_layout, InitialLayoutError};
    use crate::{
        Column, ColumnMode, DockMetrics, DockSide, Node, PanelConstraints, PanelKey, Point, Profile,
        Rect, Size,
    };

    fn baseline_panel_constraints() -> PanelConstraints {
        PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false)
    }

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
        assert!(matches!(
            create_initial_layout(Profile::Generic, &incomplete, DockMetrics::default()),
            Err(InitialLayoutError::MissingRequiredPanel { key: "history" })
        ));

        let mut duplicate = descriptors();
        duplicate.push(duplicate[0].clone());
        assert_eq!(
            create_initial_layout(Profile::Generic, &duplicate, DockMetrics::default()).unwrap_err(),
            InitialLayoutError::DuplicatePanelKey
        );
    }

    #[test]
    fn factory_errors_preserve_failure_categories() {
        let mut incomplete = descriptors();
        incomplete.retain(|(key, _)| key.as_str() != "history");
        assert!(matches!(
            create_initial_layout(Profile::Generic, &incomplete, DockMetrics::default()),
            Err(InitialLayoutError::MissingRequiredPanel { key: "history" })
        ));

        let mut duplicate = descriptors();
        duplicate.push(duplicate[0].clone());
        assert!(matches!(
            create_initial_layout(Profile::Generic, &duplicate, DockMetrics::default()),
            Err(InitialLayoutError::DuplicatePanelKey)
        ));

        let mut conflicting_widths = descriptors();
        conflicting_widths[0].1 = PanelConstraints::new(
            Size::new(400.0, 80.0),
            Size::new(420.0, 220.0),
            Some(Size::new(450.0, 300.0)),
            false,
        );
        conflicting_widths[1].1 = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(200.0, 220.0),
            Some(Size::new(350.0, 300.0)),
            false,
        );
        assert!(matches!(
            create_initial_layout(
                Profile::Generic,
                &conflicting_widths,
                DockMetrics::default()
            ),
            Err(InitialLayoutError::ConflictingPanelWidthConstraints)
        ));

        let metrics = DockMetrics {
            default_column_width: f32::NAN,
            ..DockMetrics::default()
        };
        assert!(matches!(
            create_initial_layout(Profile::Generic, &descriptors(), metrics),
            Err(InitialLayoutError::InvalidMetrics { .. })
        ));

        let mut invalid_constraints = descriptors();
        invalid_constraints[0].1 = PanelConstraints::new(
            Size::new(400.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );
        assert!(matches!(
            create_initial_layout(
                Profile::Generic,
                &invalid_constraints,
                DockMetrics::default()
            ),
            Err(InitialLayoutError::InvalidPanelConstraints { .. })
        ));
    }

    #[test]
    fn factory_rejects_invalid_metrics_and_constraints() {
        let metrics = DockMetrics {
            default_column_width: f32::NAN,
            ..DockMetrics::default()
        };
        assert!(create_initial_layout(Profile::Generic, &descriptors(), metrics).is_err());

        let mut items = descriptors();
        items[0].1 =
            PanelConstraints::new(Size::new(400.0, 80.0), Size::new(280.0, 220.0), None, false);
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
    #[test]
    fn factory_expands_column_to_fit_panel_minimum_width_for_both_profiles() {
        let metrics = DockMetrics::default();
        let wide_constraints =
            PanelConstraints::new(Size::new(400.0, 80.0), Size::new(450.0, 220.0), None, false);
        let items = descriptors()
            .into_iter()
            .map(|(key, _)| (key, wide_constraints))
            .collect::<Vec<_>>();

        for profile in [Profile::Generic, Profile::Photoshop] {
            let layout = create_initial_layout(profile, &items, metrics).unwrap();
            let root = layout.main_area(DockSide::Right).unwrap().root;
            let Node::Column(column) = layout.node(root).unwrap() else {
                panic!("main root must be a Column");
            };

            assert_eq!(column.mode, ColumnMode::Expanded);
            assert_eq!(column.width, 400.0);
            assert_eq!(column.last_expanded_width, Some(400.0));
            assert_eq!(layout.validate(&metrics), Ok(()));
            layout
                .compute_rects(
                    Rect::new(Point::new(0.0, 0.0), Size::new(800.0, 600.0)),
                    &metrics,
                )
                .expect("factory layout must be placeable at its required minimum width");
        }
    }

    #[test]
    fn factory_rejects_conflicting_panel_minimum_and_tab_maximum_widths() {
        let mut items = descriptors();
        items[0].1 = PanelConstraints::new(
            Size::new(400.0, 80.0),
            Size::new(420.0, 220.0),
            Some(Size::new(450.0, 300.0)),
            false,
        );
        items[1].1 = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(200.0, 220.0),
            Some(Size::new(350.0, 300.0)),
            false,
        );

        for profile in [Profile::Generic, Profile::Photoshop] {
            assert_eq!(
                create_initial_layout(profile, &items, DockMetrics::default()).unwrap_err(),
                InitialLayoutError::ConflictingPanelWidthConstraints
            );
        }
    }

}
