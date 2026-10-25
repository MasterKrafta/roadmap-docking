/// Failures returned by checked logical-pixel geometry operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GeometryError {
    /// A coordinate, size, or intermediate result is NaN or infinite.
    #[error("geometry contains a non-finite value")]
    NonFinite,
    /// A dimension is negative.
    #[error("geometry contains a negative dimension")]
    NegativeDimension,
    /// A required-positive dimension or effective edge is zero or negative.
    #[error("geometry requires a positive dimension or representable edge")]
    NonPositiveDimension,
    /// Arithmetic overflowed into a non-finite floating-point result.
    #[error("geometry arithmetic overflowed")]
    ArithmeticOverflow,
    /// Minimum, preferred, and maximum constraints contradict one another.
    #[error("geometry constraints are inconsistent")]
    InconsistentConstraints,
}
