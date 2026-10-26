use super::*;

impl Layout {
    pub(super) fn effective_column_width(
        requested_width: f32,
        mode: ColumnMode,
        min_width: f32,
        max_width: f32,
        metrics: &DockMetrics,
    ) -> Result<f32, CommandError> {
        let actual_width = match mode {
            ColumnMode::Expanded => requested_width.clamp(min_width, max_width),
            ColumnMode::IconsAndLabels => metrics.min_column_width,
            ColumnMode::IconsOnly => metrics.rail_width,
        };
        if !actual_width.is_finite() || actual_width <= 0.0 {
            return Err(CommandError::InvalidValue {
                field: "width",
                reason: "effective width is not finite and positive",
            });
        }
        Ok(actual_width)
    }
}
