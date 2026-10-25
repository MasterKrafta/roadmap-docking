use super::*;

pub(super) fn distribute_heights_with_minimums(
    available_height: f32,
    minimums: &[f32],
    weights: &[f32],
) -> Result<Vec<f32>, CommandError> {
    if !available_height.is_finite() || available_height <= 0.0 {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "available group height must be finite and positive",
        });
    }
    if minimums.len() != weights.len() {
        return Err(CommandError::InvalidValue {
            field: "weights",
            reason: "group weights must match expanded group count",
        });
    }
    if minimums
        .iter()
        .any(|value| !value.is_finite() || *value <= 0.0)
        || weights
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(CommandError::InvalidValue {
            field: "weights",
            reason: "group minimums must be positive and weights finite/non-negative",
        });
    }
    if minimums.is_empty() {
        return Ok(Vec::new());
    }

    let minimum_total: f32 = minimums.iter().sum();
    if !minimum_total.is_finite() || minimum_total > available_height {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "available height is below the sum of expanded group minimums",
        });
    }

    let mut result = vec![0.0; minimums.len()];
    let mut active: Vec<usize> = (0..minimums.len()).collect();
    let mut remaining = available_height;
    while !active.is_empty() {
        let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
        let equal_share = weight_sum <= f32::EPSILON;
        let proposed: Vec<(usize, f32)> = active
            .iter()
            .map(|index| {
                let height = if equal_share {
                    remaining / active.len() as f32
                } else {
                    remaining * weights[*index] / weight_sum
                };
                (*index, height)
            })
            .collect();
        let mut constrained = Vec::new();
        for (index, height) in proposed {
            if height < minimums[index] {
                result[index] = minimums[index];
                remaining -= minimums[index];
                constrained.push(index);
            }
        }
        if constrained.is_empty() {
            let weight_sum: f32 = active.iter().map(|index| weights[*index]).sum();
            let equal_share = weight_sum <= f32::EPSILON;
            for index in &active {
                result[*index] = if equal_share {
                    remaining / active.len() as f32
                } else {
                    remaining * weights[*index] / weight_sum
                };
            }
            break;
        }
        active.retain(|index| !constrained.contains(index));
    }

    if result
        .iter()
        .any(|height| !height.is_finite() || *height <= 0.0)
    {
        return Err(CommandError::InvalidValue {
            field: "available_height",
            reason: "computed group heights are not finite and positive",
        });
    }
    Ok(result)
}

pub(super) fn side_index(side: DockSide) -> usize {
    match side {
        DockSide::Left => 0,
        DockSide::Right => 1,
    }
}

pub(super) fn ratio_issues(ratios: &[f32], expected_len: usize) -> Vec<&'static str> {
    let mut issues = Vec::new();
    if ratios.len() != expected_len {
        issues.push("ratio count must match child/group count");
    }
    if ratios
        .iter()
        .any(|ratio| !ratio.is_finite() || *ratio < 0.0)
    {
        issues.push("ratios must be finite and non-negative");
    }
    let sum: f32 = ratios.iter().copied().sum();
    if !sum.is_finite() || (sum - 1.0).abs() > RATIO_TOLERANCE {
        issues.push("ratios must sum to 1.0 within 1e-4");
    }
    issues
}

pub(super) fn is_normalizable_validation_error(error: &ValidationError) -> bool {
    match error {
        ValidationError::InvalidSplit { reason, .. } => {
            *reason == "must contain at least two children" || is_ratio_error(reason)
        }
        ValidationError::InvalidTabs { reason, .. } => {
            *reason == "must contain at least one child"
                || *reason == "active_index is outside children"
        }
        ValidationError::InvalidColumn { reason, .. } => {
            *reason == "must contain at least one group before normalization"
                || is_ratio_error(reason)
        }
        // In a candidate transaction a move may temporarily create an empty
        // source Column while the destination creates another Column. Allow
        // normalization to remove empty columns first; final validation still
        // rejects any layout that exceeds the limit after cleanup.
        ValidationError::TooManyColumns { .. } => true,
        // Flyout is transient and may be closed if the persistent tree changes.
        ValidationError::InvalidFlyout { .. } => true,
        // All other errors describe broken references, invalid metadata, or
        // profile constraints that normalization must not guess how to repair.
        _ => false,
    }
}

pub(super) fn is_ratio_error(reason: &str) -> bool {
    matches!(
        reason,
        "ratio count must match child/group count"
            | "ratios must be finite and non-negative"
            | "ratios must sum to 1.0 within 1e-4"
    )
}

pub(super) fn normalized_ratios(ratios: Vec<Option<f32>>) -> Vec<f32> {
    let len = ratios.len();
    if len == 0 {
        return Vec::new();
    }

    let values: Option<Vec<f32>> =
        ratios
            .into_iter()
            .collect::<Option<Vec<f32>>>()
            .filter(|values| {
                values
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0)
            });

    let Some(values) = values else {
        return equal_ratios(len);
    };
    let sum: f32 = values.iter().copied().sum();
    if !sum.is_finite() || sum <= f32::EPSILON {
        return equal_ratios(values.len());
    }

    // Leave already-normalized ratios alone to make repeated normalization
    // exactly idempotent rather than introducing floating-point drift.
    if (sum - 1.0).abs() <= RATIO_TOLERANCE {
        return values;
    }

    let normalized: Vec<f32> = values.iter().map(|value| *value / sum).collect();
    if normalized.iter().all(|value| value.is_finite()) {
        normalized
    } else {
        equal_ratios(values.len())
    }
}

pub(super) fn equal_ratios(len: usize) -> Vec<f32> {
    if len == 0 {
        return Vec::new();
    }
    vec![1.0 / len as f32; len]
}

/// Computes sibling ratios when a new item takes a share of an existing target item.
/// Invalid old ratios fall back to equal weights, matching normalization behavior.
pub(super) fn split_sibling_ratios(
    current_ratios: &[f32],
    sibling_count: usize,
    target_index: usize,
    insert_at: usize,
    ratio: f32,
) -> Vec<f32> {
    let current_sum: f32 = current_ratios.iter().copied().sum();
    let has_valid_ratios = current_ratios.len() == sibling_count
        && current_ratios
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
        && current_sum.is_finite()
        && current_sum > f32::EPSILON;
    let mut weights = if has_valid_ratios {
        normalized_ratios(current_ratios.iter().copied().map(Some).collect())
    } else {
        equal_ratios(sibling_count)
    };

    let target_weight = weights[target_index];
    weights[target_index] = target_weight * (1.0 - ratio);
    weights.insert(insert_at, target_weight * ratio);
    normalized_ratios(weights.into_iter().map(Some).collect())
}

pub(super) fn inspect_graph(
    nodes: &SlotMap<NodeId, Node>,
    node_id: NodeId,
    parent: Option<NodeId>,
    visited: &mut HashSet<NodeId>,
    visiting: &mut HashSet<NodeId>,
    incoming: &mut HashMap<NodeId, usize>,
    errors: &mut Vec<ValidationError>,
) {
    if !nodes.contains_key(node_id) {
        if let Some(owner) = parent {
            errors.push(ValidationError::MissingNode {
                owner: Some(owner),
                missing: node_id,
            });
        }
        return;
    }
    if visiting.contains(&node_id) {
        errors.push(ValidationError::NodeCycle {
            from: parent.unwrap_or(node_id),
            to: node_id,
        });
        return;
    }
    if visited.contains(&node_id) {
        return;
    }

    visiting.insert(node_id);
    if let Some(node) = nodes.get(node_id) {
        for child in node.children() {
            let child = *child;
            if nodes.contains_key(child) {
                *incoming.entry(child).or_default() += 1;
                inspect_graph(
                    nodes,
                    child,
                    Some(node_id),
                    visited,
                    visiting,
                    incoming,
                    errors,
                );
            } else {
                errors.push(ValidationError::MissingNode {
                    owner: Some(node_id),
                    missing: child,
                });
            }
        }
    }
    visiting.remove(&node_id);
    visited.insert(node_id);
}

pub(super) fn mark_reachable(
    nodes: &SlotMap<NodeId, Node>,
    node_id: NodeId,
    reachable: &mut HashSet<NodeId>,
) {
    if !nodes.contains_key(node_id) || !reachable.insert(node_id) {
        return;
    }
    if let Some(node) = nodes.get(node_id) {
        for child in node.children() {
            mark_reachable(nodes, *child, reachable);
        }
    }
}

pub(super) fn validate_photoshop_main_root(
    nodes: &SlotMap<NodeId, Node>,
    area: DockAreaId,
    root: NodeId,
    errors: &mut Vec<ValidationError>,
) -> usize {
    match nodes.get(root) {
        Some(Node::Column(_)) => 1,
        Some(Node::Split(split)) if split.axis == Axis::X => {
            let mut count = 0;
            let mut invalid_child = false;
            for child in &split.children {
                match nodes.get(*child) {
                    Some(Node::Column(_)) => count += 1,
                    Some(_) => invalid_child = true,
                    None => {}
                }
            }
            if invalid_child {
                errors.push(ValidationError::InvalidMainRoot {
                    area,
                    root,
                    reason: "Split(X) main root children must all be Column nodes",
                });
            }
            count
        }
        Some(Node::Split(_)) => {
            errors.push(ValidationError::InvalidMainRoot {
                area,
                root,
                reason: "main Split must use axis X",
            });
            0
        }
        Some(_) => {
            errors.push(ValidationError::InvalidMainRoot {
                area,
                root,
                reason: "main root must be a Column or Split(X) of Columns",
            });
            0
        }
        None => 0,
    }
}

pub(super) fn validate_photoshop_floating_root(
    nodes: &SlotMap<NodeId, Node>,
    group: FloatingGroupId,
    root: NodeId,
    errors: &mut Vec<ValidationError>,
) {
    match nodes.get(root) {
        Some(Node::Tabs(_)) => {}
        Some(Node::Column(column)) => {
            let invalid_child = column.groups.iter().any(|child| {
                nodes.contains_key(*child) && !matches!(nodes.get(*child), Some(Node::Tabs(_)))
            });
            if invalid_child {
                errors.push(ValidationError::InvalidFloatingRoot {
                    group,
                    root,
                    reason: "floating Column children must all be Tabs nodes",
                });
            }
        }
        Some(Node::Split(split)) if split.axis == Axis::X => {
            let invalid_child = split.children.iter().any(|child| {
                nodes.contains_key(*child) && !matches!(nodes.get(*child), Some(Node::Column(_)))
            });
            if invalid_child {
                errors.push(ValidationError::InvalidFloatingRoot {
                    group,
                    root,
                    reason: "Split(X) floating composition children must all be Column nodes",
                });
            }
        }
        Some(Node::Split(split)) if split.axis == Axis::Y => {
            let invalid_child = split.children.iter().any(|child| {
                nodes.contains_key(*child) && !matches!(nodes.get(*child), Some(Node::Tabs(_)))
            });
            if invalid_child {
                errors.push(ValidationError::InvalidFloatingRoot {
                    group,
                    root,
                    reason: "Split(Y) floating root children must all be Tabs nodes",
                });
            }
        }
        Some(Node::Split(_)) => errors.push(ValidationError::InvalidFloatingRoot {
            group,
            root,
            reason: "floating composition Split must use axis X or floating stack Split must use axis Y",
        }),
        Some(_) => errors.push(ValidationError::InvalidFloatingRoot {
            group,
            root,
            reason: "floating root must be Tabs, Column, Split(X) of Columns, or Split(Y) of Tabs",
        }),
        None => {}
    }
}

pub(super) fn geometry_error_reason(error: GeometryError) -> &'static str {
    match error {
        GeometryError::NonFinite => "bounds contain a non-finite value",
        GeometryError::NegativeDimension => "bounds dimensions must not be negative",
        GeometryError::NonPositiveDimension => "bounds must have positive dimensions",
        GeometryError::ArithmeticOverflow => "bounds arithmetic overflowed",
        GeometryError::InconsistentConstraints => "geometry constraints are inconsistent",
    }
}
