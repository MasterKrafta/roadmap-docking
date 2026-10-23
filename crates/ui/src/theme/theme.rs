use super::{ThemeColors, ThemeMetrics, ThemeTypography};

/// The single application-wide theme. It contains no docking model state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Theme {
    pub colors: ThemeColors,
    pub metrics: ThemeMetrics,
    pub typography: ThemeTypography,
}

