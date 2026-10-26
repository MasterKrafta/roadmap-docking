use super::helpers::*;
use super::*;

impl Layout {
    /// Validates the entire model and returns every detected issue.
    ///
    /// Metrics are supplied explicitly so callers and tests use the same
    /// thresholds as the core algorithms. An empty layout is valid: the canvas
    /// always exists, while empty main dock areas are represented by None.
    pub fn validate(&self, metrics: &DockMetrics) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        let metrics_result = metrics.validate();
        if let Err(reason) = metrics_result {
            errors.push(ValidationError::InvalidMetrics { reason });
        }
        let metrics_are_valid = metrics_result.is_ok();

        if let Err(reason) = self.canvas.validate() {
            errors.push(ValidationError::InvalidCanvas {
                reason: geometry_error_reason(reason),
            });
        }
        let panel_placements =
            self.validate_node_records(metrics, metrics_are_valid, &mut errors);
        self.validate_panel_instances(&panel_placements, &mut errors);

        let roots = self.validate_dock_areas(metrics, metrics_are_valid, &mut errors);

        self.validate_flyout(&mut errors);

        if !self.validate_tree_graph(&roots, &mut errors) {
            return Err(errors);
        }

        self.validate_expanded_column_widths(metrics, metrics_are_valid, &mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
