use proptest::prelude::*;
use proptest::test_runner::{Config as ProptestConfig, FileFailurePersistence};

use crate::{
    ColumnRef, DockCommand, DockMetrics, DockSide, DropTarget, DropZone, FloatSource,
    FloatingGroupId, Layout, Node, NodeId, PanelConstraints, PanelId, PanelKey, Point, Rect,
    Size, SplitSide,
};
#[cfg(feature = "serde")]
use crate::{ColumnMode, PanelState, Profile};

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
    Layout::initial_photoshop(metrics).expect("default Photoshop layout is valid")
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::new(Point::new(x, y), Size::new(width, height))
}

fn dock_groups(layout: &Layout) -> Vec<NodeId> {
    layout
        .dock_areas()
        .iter()
        .flat_map(|area| area.columns.iter())
        .flat_map(|column| column.groups.iter().copied())
        .filter(|group| matches!(layout.nodes().get(*group), Some(Node::Tabs(_))))
        .collect()
}

fn docked_panels(layout: &Layout) -> Vec<PanelId> {
    dock_groups(layout)
        .into_iter()
        .flat_map(|group| match layout.nodes().get(group) {
            Some(Node::Tabs(tabs)) => tabs.panels.clone(),
            Some(Node::Split(_)) | None => Vec::new(),
        })
        .collect()
}

fn columns(layout: &Layout) -> Vec<ColumnRef> {
    layout
        .dock_areas()
        .iter()
        .flat_map(|area| {
            (0..area.columns.len()).map(move |index| ColumnRef::new(area.side, index))
        })
        .collect()
}

fn adjacent_column_pairs(layout: &Layout) -> Vec<(ColumnRef, ColumnRef)> {
    layout
        .dock_areas()
        .iter()
        .flat_map(|area| {
            (0..area.columns.len().saturating_sub(1)).map(move |index| {
                (
                    ColumnRef::new(area.side, index),
                    ColumnRef::new(area.side, index + 1),
                )
            })
        })
        .collect()
}

fn choose<T: Copy>(items: &[T], selector: u8) -> T {
    items[selector as usize % items.len()]
}

/// Commands use IDs collected from the current state. A valid ID can still be rejected
/// because a dynamic constraint, such as the column limit, has become active.
fn generated_command(
    layout: &Layout,
    operation: u8,
    a: u8,
    b: u8,
    step_index: usize,
    metrics: &DockMetrics,
) -> Option<DockCommand> {
    let groups = dock_groups(layout);
    let panels: Vec<PanelId> = layout.panels().keys().collect();
    let docked_panels = docked_panels(layout);
    let columns = columns(layout);
    let floating: Vec<FloatingGroupId> = layout.floating_groups().keys().collect();
    let pairs = adjacent_column_pairs(layout);

    match operation % 16 {
        0 => {
            if groups.is_empty() {
                return None;
            }
            let group = choose(&groups, a);
            let Some(Node::Tabs(tabs)) = layout.nodes().get(group) else {
                return None;
            };
            Some(DockCommand::ActivateTab {
                group,
                index: b as usize % tabs.panels.len(),
            })
        }
        1 => {
            if groups.is_empty() {
                return None;
            }
            let group = choose(&groups, a);
            let Some(Node::Tabs(tabs)) = layout.nodes().get(group) else {
                return None;
            };
            let len = tabs.panels.len();
            Some(DockCommand::ReorderTab {
                group,
                from: b as usize % len,
                to: a as usize % len,
            })
        }
        2 => {
            if panels.is_empty() {
                return None;
            }
            let panel = choose(&panels, a);
            let target = if !floating.is_empty() && (groups.is_empty() || b % 2 == 1) {
                DropTarget::FloatingGroup {
                    floating: choose(&floating, b),
                    zone: DropZone::Center,
                }
            } else if !groups.is_empty() {
                DropTarget::Group {
                    group: choose(&groups, b),
                    zone: DropZone::Center,
                }
            } else {
                return None;
            };
            Some(DockCommand::MovePanel { panel, target })
        }
        3 => {
            if groups.is_empty() {
                return None;
            }
            let group = choose(&groups, a);
            let Some(Node::Tabs(tabs)) = layout.nodes().get(group) else {
                return None;
            };
            let panel = choose(&tabs.panels, b);
            let side = match a % 4 {
                0 => SplitSide::Top,
                1 => SplitSide::Bottom,
                2 => SplitSide::Left,
                _ => SplitSide::Right,
            };
            Some(DockCommand::SplitGroup { panel, group, side })
        }
        4 => {
            let valid_columns: Vec<ColumnRef> = columns
                .iter()
                .copied()
                .filter(|column_ref| {
                    layout
                        .dock_areas()
                        .iter()
                        .find(|area| area.side == column_ref.side)
                        .and_then(|area| area.columns.get(column_ref.index))
                        .is_some_and(|column| column.groups.len() >= 2)
                })
                .collect();
            if valid_columns.is_empty() {
                return None;
            }
            let column = choose(&valid_columns, a);
            let group_count = layout
                .dock_areas()
                .iter()
                .find(|area| area.side == column.side)
                .and_then(|area| area.columns.get(column.index))
                .map_or(2, |column| column.groups.len());
            Some(DockCommand::ResizeGroups {
                column,
                boundary: b as usize % (group_count - 1),
                delta: (i16::from(a % 81) - 40) as f32,
            })
        }
        5 => {
            if columns.is_empty() {
                return None;
            }
            Some(DockCommand::ResizeColumn {
                column: choose(&columns, a),
                width: metrics.min_column_width + f32::from(b % 280),
            })
        }
        6 => {
            if pairs.is_empty() {
                return None;
            }
            let (first, second) = choose(&pairs, a);
            Some(DockCommand::ResizeColumnBoundary {
                first,
                second,
                delta: (i16::from(b % 41) - 20) as f32,
            })
        }
        7 => {
            if docked_panels.is_empty() {
                return None;
            }
            Some(DockCommand::Float {
                source: FloatSource::Panel(choose(&docked_panels, a)),
                rect: rect(f32::from(a), f32::from(b), 480.0, 320.0),
            })
        }
        8 => {
            if groups.is_empty() {
                return None;
            }
            Some(DockCommand::Float {
                source: FloatSource::Group(choose(&groups, a)),
                rect: rect(f32::from(a), f32::from(b), 520.0, 360.0),
            })
        }
        9 => {
            if floating.is_empty() {
                return None;
            }
            let floating_id = choose(&floating, a);
            let bounds = layout.floating_groups().get(floating_id)?.bounds;
            Some(DockCommand::MoveFloating {
                floating: floating_id,
                rect: rect(
                    bounds.origin.x + f32::from(a % 17) - 8.0,
                    bounds.origin.y + f32::from(b % 17) - 8.0,
                    bounds.size.width,
                    bounds.size.height,
                ),
            })
        }
        10 => {
            if floating.is_empty() {
                return None;
            }
            let floating_id = choose(&floating, a);
            let bounds = layout.floating_groups().get(floating_id)?.bounds;
            Some(DockCommand::ResizeFloating {
                floating: floating_id,
                rect: rect(
                    bounds.origin.x,
                    bounds.origin.y,
                    bounds.size.width + f32::from(a % 41),
                    bounds.size.height + f32::from(b % 41),
                ),
            })
        }
        11 => {
            if floating.is_empty() {
                return None;
            }
            Some(DockCommand::RaiseFloating {
                floating: choose(&floating, a),
            })
        }
        12 => {
            if floating.is_empty() {
                return None;
            }
            let floating_id = choose(&floating, a);
            let target = if !groups.is_empty() && b % 2 == 0 {
                DropTarget::Group {
                    group: choose(&groups, b),
                    zone: DropZone::Center,
                }
            } else {
                DropTarget::DockEdge {
                    side: if b % 2 == 0 {
                        DockSide::Left
                    } else {
                        DockSide::Right
                    },
                }
            };
            Some(DockCommand::DockFloating {
                floating: floating_id,
                target,
            })
        }
        13 => {
            if panels.is_empty() {
                return None;
            }
            Some(DockCommand::ClosePanel {
                panel: choose(&panels, a),
            })
        }
        14 => {
            if groups.is_empty() {
                return None;
            }
            Some(DockCommand::CloseGroup {
                group: choose(&groups, a),
            })
        }
        15 => {
            let key = PanelKey::new(&format!("generated_{step_index}"))
                .expect("generated panel key is valid");
            let constraints = PanelConstraints::new(
                Size::new(metrics.min_column_width, metrics.min_group_height),
                Size::new(metrics.default_column_width, metrics.min_group_height),
                Size::new(f32::MAX, f32::MAX),
                false,
            );
            let target = if groups.is_empty() {
                None
            } else {
                Some(DropTarget::Group {
                    group: choose(&groups, a),
                    zone: DropZone::Center,
                })
            };
            Some(DockCommand::OpenPanel {
                key,
                constraints,
                target,
            })
        }
        _ => None,
    }
}

#[cfg(feature = "serde")]
#[derive(Clone, Debug, PartialEq)]
struct LogicalPanel {
    key: String,
    constraints: PanelConstraints,
    state: PanelState,
}

#[cfg(feature = "serde")]
#[derive(Clone, Debug, PartialEq)]
struct LogicalGroup {
    panels: Vec<String>,
    active_index: usize,
}

#[cfg(feature = "serde")]
#[derive(Clone, Debug, PartialEq)]
struct LogicalColumn {
    width: f32,
    mode: ColumnMode,
    groups: Vec<LogicalGroup>,
    group_ratios: Vec<f32>,
}

#[cfg(feature = "serde")]
#[derive(Clone, Debug, PartialEq)]
struct LogicalDockArea {
    side: DockSide,
    columns: Vec<LogicalColumn>,
}

#[cfg(feature = "serde")]
#[derive(Clone, Debug, PartialEq)]
struct LogicalFloatingGroup {
    panels: Vec<String>,
    active_index: usize,
    bounds: Rect,
    z_order: u64,
}

#[cfg(feature = "serde")]
#[derive(Clone, Debug, PartialEq)]
struct LogicalLayout {
    profile: Profile,
    canvas_bounds: Rect,
    panels: Vec<LogicalPanel>,
    dock_areas: Vec<LogicalDockArea>,
    floating_groups: Vec<LogicalFloatingGroup>,
}

#[cfg(feature = "serde")]
fn logical_panel_key(layout: &Layout, panel: PanelId) -> String {
    layout
        .panels()
        .get(panel)
        .expect("all placed panels exist in the panel registry")
        .key()
        .as_str()
        .to_owned()
}

#[cfg(feature = "serde")]
fn logical_group(layout: &Layout, group: NodeId) -> LogicalGroup {
    match layout.nodes().get(group).expect("group node exists") {
        Node::Tabs(tabs) => LogicalGroup {
            panels: tabs
                .panels
                .iter()
                .map(|panel| logical_panel_key(layout, *panel))
                .collect(),
            active_index: tabs.active_index,
        },
        Node::Split(_) => panic!("Photoshop profile does not allow nested split groups"),
    }
}

#[cfg(feature = "serde")]
fn logical_snapshot(layout: &Layout) -> LogicalLayout {
    let mut panels: Vec<LogicalPanel> = layout
        .panels()
        .values()
        .map(|panel| LogicalPanel {
            key: panel.key().as_str().to_owned(),
            constraints: *panel.constraints(),
            state: *panel.state(),
        })
        .collect();
    panels.sort_by(|left, right| left.key.cmp(&right.key));

    let dock_areas = layout
        .dock_areas()
        .iter()
        .map(|area| LogicalDockArea {
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
                        .map(|group| logical_group(layout, *group))
                        .collect(),
                    group_ratios: column.group_ratios.clone(),
                })
                .collect(),
        })
        .collect();

    let mut floating_groups: Vec<LogicalFloatingGroup> = layout
        .floating_groups()
        .values()
        .map(|floating| LogicalFloatingGroup {
            panels: floating
                .panels
                .iter()
                .map(|panel| logical_panel_key(layout, *panel))
                .collect(),
            active_index: floating.active_index,
            bounds: floating.bounds,
            z_order: floating.z_order,
        })
        .collect();
    floating_groups.sort_by_key(|floating| floating.z_order);

    LogicalLayout {
        profile: layout.profile(),
        canvas_bounds: layout.canvas().bounds,
        panels,
        dock_areas,
        floating_groups,
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 96,
        failure_persistence: Some(Box::new(FileFailurePersistence::SourceParallel(
            "tests/regressions",
        ))),
        .. ProptestConfig::default()
    })]

    #[test]
    fn random_command_sequences_preserve_invariants_and_replay(
        steps in prop::collection::vec((any::<u8>(), any::<u8>(), any::<u8>()), 1..80)
    ) {
        let metrics = metrics();
        let mut layout = initial_layout(&metrics);
        let mut journal: Vec<DockCommand> = Vec::new();

        for (step_index, (operation, a, b)) in steps.iter().copied().enumerate() {
            let Some(command) = generated_command(&layout, operation, a, b, step_index, &metrics) else {
                continue;
            };

            let before = format!("{layout:?}");
            let version_before = layout.version();

            match layout.apply(command.clone(), &metrics) {
                Ok(_) => {
                    journal.push(command);
                    prop_assert!(
                        layout.validate(&metrics).is_ok(),
                        "layout must validate after every successful command; step={step_index}"
                    );
                }
                Err(_) => {
                    prop_assert_eq!(layout.version(), version_before,
                        "a rejected command must not increment the version");
                    prop_assert_eq!(format!("{layout:?}"), before,
                        "a rejected command must leave the complete layout unchanged");
                    prop_assert!(layout.validate(&metrics).is_ok(),
                        "layout must remain valid after a rejected command");
                }
            }
        }

        #[cfg(feature = "serde")]
        {
            let encoded = serde_json::to_string(&journal)
                .expect("the command journal should serialize");
            let decoded: Vec<DockCommand> = serde_json::from_str(&encoded)
                .expect("the command journal should deserialize");

            let mut replay = initial_layout(&metrics);
            for (index, command) in decoded.into_iter().enumerate() {
                replay.apply(command, &metrics)
                    .unwrap_or_else(|error| panic!("journal command {index} failed to replay: {error}"));
                prop_assert!(replay.validate(&metrics).is_ok(),
                    "replayed layout must validate after command {index}");
            }

            prop_assert_eq!(
                logical_snapshot(&replay),
                logical_snapshot(&layout),
                "serialized journal replay must reproduce logical state, ignoring IDs and version"
            );
        }
    }

    #[test]
    fn rejected_commands_are_fully_transactional(selector in any::<u8>()) {
        let metrics = metrics();
        let mut layout = initial_layout(&metrics);
        let groups = dock_groups(&layout);
        let group = groups[selector as usize % groups.len()];
        let Node::Tabs(tabs) = layout.nodes().get(group).expect("selected group exists") else {
            unreachable!();
        };

        let command = if selector % 2 == 0 {
            DockCommand::ActivateTab {
                group,
                index: tabs.panels.len(),
            }
        } else {
            DockCommand::DockFloating {
                floating: FloatingGroupId::null(),
                target: DropTarget::DockEdge { side: DockSide::Left },
            }
        };
        let before = format!("{layout:?}");
        let version_before = layout.version();

        prop_assert!(layout.apply(command, &metrics).is_err(),
            "the generated command must be rejected");
        prop_assert_eq!(layout.version(), version_before);
        prop_assert_eq!(format!("{layout:?}"), before);
        prop_assert!(layout.validate(&metrics).is_ok());
    }
}
