use dock_core::{
    Axis, ColumnMode, ColumnRef, CommandError, DockCommand, DockMetrics, DockSide, DropTarget,
    DropZone, FloatSource, FloatingGroupId, Layout, Node, NodeId, PanelConstraints, PanelId,
    PanelKey, Point, Profile, Rect, Size, SplitSide,
};
use proptest::{
    prelude::*,
    test_runner::{FileFailurePersistence, ProptestConfig},
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

fn initial_layout(metrics: &DockMetrics) -> Layout {
    Layout::initial_photoshop(metrics).expect("known-good Photoshop metrics")
}

fn dock_groups(layout: &Layout) -> Vec<NodeId> {
    let mut groups = Vec::new();
    for area in layout.dock_areas() {
        for column in &area.columns {
            for group in &column.groups {
                if matches!(layout.nodes().get(*group), Some(Node::Tabs(_))) {
                    groups.push(*group);
                }
            }
        }
    }
    groups
}

fn docked_panel_locations(layout: &Layout) -> Vec<(PanelId, NodeId)> {
    let mut panels = Vec::new();
    for area in layout.dock_areas() {
        for column in &area.columns {
            for group in &column.groups {
                if let Some(Node::Tabs(tabs)) = layout.nodes().get(*group) {
                    panels.extend(tabs.panels.iter().copied().map(|panel| (panel, *group)));
                }
            }
        }
    }
    panels
}

fn column_refs(layout: &Layout) -> Vec<ColumnRef> {
    let mut columns = Vec::new();
    for area in layout.dock_areas() {
        for index in 0..area.columns.len() {
            columns.push(ColumnRef::new(area.side, index));
        }
    }
    columns
}

fn adjacent_column_refs(layout: &Layout) -> Vec<(ColumnRef, ColumnRef)> {
    let mut pairs = Vec::new();
    for area in layout.dock_areas() {
        for index in 0..area.columns.len().saturating_sub(1) {
            pairs.push((
                ColumnRef::new(area.side, index),
                ColumnRef::new(area.side, index + 1),
            ));
        }
    }
    pairs
}

fn floating_ids(layout: &Layout) -> Vec<FloatingGroupId> {
    layout.floating_groups().iter().map(|(id, _)| id).collect()
}

fn all_panel_ids(layout: &Layout) -> Vec<PanelId> {
    layout.panels().iter().map(|(id, _)| id).collect()
}

fn group_panels(layout: &Layout, group: NodeId) -> Vec<PanelId> {
    match layout.nodes().get(group) {
        Some(Node::Tabs(tabs)) => tabs.panels.clone(),
        _ => Vec::new(),
    }
}

fn safe_rect_for_panels(
    layout: &Layout,
    metrics: &DockMetrics,
    panels: &[PanelId],
    origin: Point,
) -> Rect {
    let mut minimum_width = metrics.min_column_width;
    let mut minimum_height = metrics.min_group_height;
    for panel_id in panels {
        if let Some(panel) = layout.panels().get(*panel_id) {
            let minimum = panel.constraints().min_size();
            minimum_width = minimum_width.max(minimum.width);
            minimum_height = minimum_height.max(minimum.height);
        }
    }
    Rect::new(
        origin,
        Size::new(minimum_width + 24.0, minimum_height + 24.0),
    )
}

/// Proptest supplies operation selectors. IDs are resolved against the current
/// layout before every command, so commands do not retain stale IDs after normalization.
fn command_for(
    layout: &Layout,
    metrics: &DockMetrics,
    operation: u8,
    a: usize,
    b: usize,
    delta: f32,
) -> Option<DockCommand> {
    let groups = dock_groups(layout);
    let docked_panels = docked_panel_locations(layout);
    let columns = column_refs(layout);
    let adjacent_columns = adjacent_column_refs(layout);
    let floats = floating_ids(layout);
    let panels = all_panel_ids(layout);

    match operation {
        0 => {
            let group = *groups.get(a % groups.len().max(1))?;
            let Node::Tabs(tabs) = layout.nodes().get(group)? else {
                return None;
            };
            (!tabs.panels.is_empty()).then_some(DockCommand::ActivateTab {
                group,
                index: b % tabs.panels.len(),
            })
        }
        1 => {
            let candidates: Vec<_> = groups
                .iter()
                .copied()
                .filter(|group| group_panels(layout, *group).len() > 1)
                .collect();
            let group = *candidates.get(a % candidates.len().max(1))?;
            let len = group_panels(layout, group).len();
            Some(DockCommand::ReorderTab {
                group,
                from: a % len,
                to: b % len,
            })
        }
        2 => {
            let (panel, _) = *docked_panels.get(a % docked_panels.len().max(1))?;
            let group = *groups.get(b % groups.len().max(1))?;
            Some(DockCommand::MovePanel {
                panel,
                target: DropTarget::Group {
                    group,
                    zone: DropZone::Center,
                },
            })
        }
        3 => {
            let (panel, group) = *docked_panels.get(a % docked_panels.len().max(1))?;
            let side = match b % 4 {
                0 => SplitSide::Top,
                1 => SplitSide::Bottom,
                2 => SplitSide::Left,
                _ => SplitSide::Right,
            };
            Some(DockCommand::SplitGroup { panel, group, side })
        }
        4 => {
            let candidates: Vec<_> = layout
                .dock_areas()
                .iter()
                .flat_map(|area| {
                    area.columns.iter().enumerate().filter_map(move |(index, column)| {
                        (column.groups.len() > 1).then_some((
                            ColumnRef::new(area.side, index),
                            column.groups.len(),
                        ))
                    })
                })
                .collect();
            let (column, group_count) = *candidates.get(a % candidates.len().max(1))?;
            Some(DockCommand::ResizeGroups {
                column,
                boundary: b % (group_count - 1),
                delta,
            })
        }
        5 => {
            let column = *columns.get(a % columns.len().max(1))?;
            Some(DockCommand::ResizeColumn {
                column,
                width: metrics.min_column_width + (b % 501) as f32,
            })
        }
        6 => {
            let (first, second) =
                *adjacent_columns.get(a % adjacent_columns.len().max(1))?;
            Some(DockCommand::ResizeColumnBoundary {
                first,
                second,
                delta,
            })
        }
        7 => {
            let (panel, _) = *docked_panels.get(a % docked_panels.len().max(1))?;
            let rect = safe_rect_for_panels(
                layout,
                metrics,
                &[panel],
                Point::new(delta, delta / 2.0),
            );
            Some(DockCommand::Float {
                source: FloatSource::Panel(panel),
                rect,
            })
        }
        8 => {
            let group = *groups.get(a % groups.len().max(1))?;
            let group_panels = group_panels(layout, group);
            if group_panels.is_empty() {
                return None;
            }
            let rect = safe_rect_for_panels(
                layout,
                metrics,
                &group_panels,
                Point::new(delta, delta / 2.0),
            );
            Some(DockCommand::Float {
                source: FloatSource::Group(group),
                rect,
            })
        }
        9 => {
            let floating = *floats.get(a % floats.len().max(1))?;
            let bounds = layout.floating_groups().get(floating)?.bounds;
            Some(DockCommand::MoveFloating {
                floating,
                rect: Rect::new(Point::new(delta, delta / 2.0), bounds.size),
            })
        }
        10 => {
            let floating = *floats.get(a % floats.len().max(1))?;
            let group = layout.floating_groups().get(floating)?;
            let rect = safe_rect_for_panels(layout, metrics, &group.panels, group.bounds.origin);
            Some(DockCommand::ResizeFloating { floating, rect })
        }
        11 => {
            let floating = *floats.get(a % floats.len().max(1))?;
            Some(DockCommand::RaiseFloating { floating })
        }
        12 => {
            let floating = *floats.get(a % floats.len().max(1))?;
            let group = *groups.get(b % groups.len().max(1))?;
            Some(DockCommand::DockFloating {
                floating,
                target: DropTarget::Group {
                    group,
                    zone: DropZone::Center,
                },
            })
        }
        13 => {
            let panel = *panels.get(a % panels.len().max(1))?;
            Some(DockCommand::ClosePanel { panel })
        }
        14 => {
            let group = *groups.get(a % groups.len().max(1))?;
            Some(DockCommand::CloseGroup { group })
        }
        15 => {
            let group = *groups.get(b % groups.len().max(1))?;
            let key = PanelKey::new(&format!("prop-tool-{}", a % 8)).ok()?;
            let constraints = PanelConstraints::new(
                Size::new(80.0, 60.0),
                Size::new(180.0, 120.0),
                Size::new(1200.0, 1000.0),
                false,
            );
            Some(DockCommand::OpenPanel {
                key,
                constraints,
                target: Some(DropTarget::Group {
                    group,
                    zone: DropZone::Center,
                }),
            })
        }
        16 => {
            let panel = *panels.get(a % panels.len().max(1))?;
            let floating = *floats.get(b % floats.len().max(1))?;
            Some(DockCommand::MovePanel {
                panel,
                target: DropTarget::FloatingGroup {
                    floating,
                    zone: DropZone::Center,
                },
            })
        }
        17 => {
            let floating = *floats.get(a % floats.len().max(1))?;
            let side = if b % 2 == 0 {
                DockSide::Left
            } else {
                DockSide::Right
            };
            Some(DockCommand::DockFloating {
                floating,
                target: DropTarget::DockEdge { side },
            })
        }
        18 => {
            if floats.len() < 2 {
                return None;
            }
            let floating = floats[a % floats.len()];
            let targets: Vec<_> = floats.iter().copied().filter(|id| *id != floating).collect();
            let target = *targets.get(b % targets.len())?;
            Some(DockCommand::DockFloating {
                floating,
                target: DropTarget::FloatingGroup {
                    floating: target,
                    zone: DropZone::Center,
                },
            })
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
enum LogicalNode {
    Missing,
    Tabs {
        panel_keys: Vec<String>,
        active_index: usize,
    },
    Split {
        axis: Axis,
        children: Vec<LogicalNode>,
        ratios: Vec<f32>,
    },
}

fn logical_node(layout: &Layout, id: NodeId) -> LogicalNode {
    match layout.nodes().get(id) {
        None => LogicalNode::Missing,
        Some(Node::Tabs(tabs)) => LogicalNode::Tabs {
            panel_keys: tabs.panels.iter().map(|id| panel_key(layout, *id)).collect(),
            active_index: tabs.active_index,
        },
        Some(Node::Split(split)) => LogicalNode::Split {
            axis: split.axis,
            children: split
                .children
                .iter()
                .map(|child| logical_node(layout, *child))
                .collect(),
            ratios: split.ratios.clone(),
        },
    }
}

fn panel_key(layout: &Layout, id: PanelId) -> String {
    layout
        .panels()
        .get(id)
        .map(|panel| panel.key().as_str().to_owned())
        .unwrap_or_else(|| "<missing-panel>".to_owned())
}

#[derive(Clone, Debug, PartialEq)]
struct LogicalPanel {
    key: String,
    minimum: Size,
    preferred: Size,
    maximum: Size,
    singleton: bool,
    last_known_size: Option<Size>,
}

#[derive(Clone, Debug, PartialEq)]
struct LogicalColumn {
    width: f32,
    mode: ColumnMode,
    groups: Vec<LogicalNode>,
    group_ratios: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
struct LogicalArea {
    side: DockSide,
    columns: Vec<LogicalColumn>,
}

#[derive(Clone, Debug, PartialEq)]
struct LogicalFloatingGroup {
    panel_keys: Vec<String>,
    active_index: usize,
    bounds: Rect,
    z_order: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct LogicalSnapshot {
    version: u64,
    profile: Profile,
    canvas_bounds: Rect,
    dock_areas: Vec<LogicalArea>,
    floating_groups: Vec<LogicalFloatingGroup>,
    panels: Vec<LogicalPanel>,
    nodes: Vec<LogicalNode>,
}

/// This projection intentionally omits all SlotMap keys, so replay checks logical layout.
fn logical_snapshot(layout: &Layout) -> LogicalSnapshot {
    let dock_areas = layout
        .dock_areas()
        .iter()
        .map(|area| LogicalArea {
            side: area.side,
            columns: area
                .columns
                .iter()
                .map(|column| LogicalColumn {
                    width: column.width,
                    mode: column.mode,
                    groups: column
                        .groups
                        .iter()
                        .map(|group| logical_node(layout, *group))
                        .collect(),
                    group_ratios: column.group_ratios.clone(),
                })
                .collect(),
        })
        .collect();

    let mut floating_groups: Vec<_> = layout
        .floating_groups()
        .iter()
        .map(|(_, group)| LogicalFloatingGroup {
            panel_keys: group.panels.iter().map(|panel| panel_key(layout, *panel)).collect(),
            active_index: group.active_index,
            bounds: group.bounds,
            z_order: group.z_order,
        })
        .collect();
    floating_groups.sort_by_key(|group| format!("{group:?}"));

    let mut panels: Vec<_> = layout
        .panels()
        .iter()
        .map(|(_, panel)| {
            let constraints = panel.constraints();
            LogicalPanel {
                key: panel.key().as_str().to_owned(),
                minimum: constraints.min_size(),
                preferred: constraints.preferred_size(),
                maximum: constraints.max_size(),
                singleton: constraints.is_singleton(),
                last_known_size: panel.state().last_known_size(),
            }
        })
        .collect();
    panels.sort_by_key(|panel| format!("{panel:?}"));

    let mut nodes: Vec<_> = layout
        .nodes()
        .iter()
        .map(|(id, _)| logical_node(layout, id))
        .collect();
    nodes.sort_by_key(|node| format!("{node:?}"));

    LogicalSnapshot {
        version: layout.version(),
        profile: layout.profile(),
        canvas_bounds: layout.canvas().bounds,
        dock_areas,
        floating_groups,
        panels,
        nodes,
    }
}

#[test]
fn regression_rejected_unknown_group_command_is_atomic() {
    let metrics = metrics();
    let mut layout = initial_layout(&metrics);
    let before = logical_snapshot(&layout);

    let error = layout
        .apply(
            DockCommand::ActivateTab {
                group: NodeId::null(),
                index: 0,
            },
            &metrics,
        )
        .expect_err("null NodeId must be rejected");

    assert!(matches!(error, CommandError::UnknownGroup(_)));
    assert_eq!(logical_snapshot(&layout), before);
    assert!(layout.validate(&metrics).is_ok());
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        max_shrink_iters: 4096,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/regressions/command_sequences.txt"
        )))),
        .. ProptestConfig::default()
    })]

    #[test]
    fn generated_command_sequences_preserve_invariants_and_replay(
        steps in prop::collection::vec(
            (0u8..19, any::<usize>(), any::<usize>(), -600.0f32..600.0f32),
            1..70
        )
    ) {
        let metrics = metrics();
        let initial = initial_layout(&metrics);
        let mut layout = initial.clone();
        let mut journal: Vec<(DockCommand, Result<(), CommandError>)> = Vec::new();

        for (operation, a, b, delta) in steps {
            let Some(command) = command_for(&layout, &metrics, operation, a, b, delta) else {
                continue;
            };
            let before = logical_snapshot(&layout);
            let expected_result = match layout.apply(command.clone(), &metrics) {
                Ok(_) => {
                    prop_assert!(
                        layout.validate(&metrics).is_ok(),
                        "successful command left an invalid layout: {command:?}"
                    );
                    Ok(())
                }
                Err(error) => {
                    prop_assert_eq!(
                        logical_snapshot(&layout),
                        before,
                        "rejected command changed the logical layout: {command:?}"
                    );
                    Err(error)
                }
            };
            journal.push((command, expected_result));
        }

        #[cfg(feature = "serde")]
        {
            let serialized = serde_json::to_string(&journal)
                .expect("command journal serializes with the serde feature");
            let decoded: Vec<(DockCommand, Result<(), CommandError>)> =
                serde_json::from_str(&serialized)
                    .expect("serialized command journal can be deserialized");

            let mut replayed = initial_layout(&metrics);
            for (command, expected_result) in decoded {
                let actual_result = match replayed.apply(command.clone(), &metrics) {
                    Ok(_) => Ok(()),
                    Err(error) => Err(error),
                };
                let succeeded = actual_result.is_ok();
                prop_assert_eq!(
                    actual_result,
                    expected_result.clone(),
                    "journal replay returned a different result for {command:?}"
                );
                if succeeded {
                    prop_assert!(
                        replayed.validate(&metrics).is_ok(),
                        "replayed command left an invalid layout: {command:?}"
                    );
                }
            }

            prop_assert_eq!(
                logical_snapshot(&layout),
                logical_snapshot(&replayed),
                "replaying the serialized journal must reproduce the logical layout"
            );
        }
    }
}
