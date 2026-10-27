use super::*;

pub(super) use super::ratio_helpers::{
    equal_ratios, is_normalizable_validation_error, normalized_ratios, ratio_issues,
    split_sibling_ratios,
};


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
