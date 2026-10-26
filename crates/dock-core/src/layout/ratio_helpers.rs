use super::*;

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
