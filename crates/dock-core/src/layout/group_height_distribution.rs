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
