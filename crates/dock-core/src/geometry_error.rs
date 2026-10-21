/// Failures returned by checked logical-pixel geometry operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    /// A coordinate, size, or intermediate result is NaN or infinite.
    NonFinite,
    /// A dimension is negative.
    NegativeDimension,
    /// A dimension that must be positive is zero or negative.
    NonPositiveDimension,
    /// Arithmetic overflowed into a non-finite floating-point result.
    ArithmeticOverflow,
    /// Minimum, preferred, and maximum constraints contradict one another.
    InconsistentConstraints,
}
