use super::{ThemeColors, ThemeMetrics, ThemeTypography};

/// The single application-wide theme. It contains no docking model state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub colors: ThemeColors,
    pub metrics: ThemeMetrics,
    pub typography: ThemeTypography,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            colors: ThemeColors::default(),
            metrics: ThemeMetrics::default(),
            typography: ThemeTypography::default(),
        }
    }
}
