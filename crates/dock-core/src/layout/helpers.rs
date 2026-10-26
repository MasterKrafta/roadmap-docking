use super::*;

pub(super) use super::ratio_helpers::{
    equal_ratios, is_normalizable_validation_error, is_ratio_error, normalized_ratios, ratio_issues,
    split_sibling_ratios,
};

pub(super) use super::graph_validation_helpers::{
    inspect_graph, mark_reachable, validate_photoshop_floating_root, validate_photoshop_main_root,
};

pub(super) use super::group_sizing::distribute_heights_with_minimums;

pub(super) fn side_index(side: DockSide) -> usize {
    match side {
        DockSide::Left => 0,
        DockSide::Right => 1,
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
