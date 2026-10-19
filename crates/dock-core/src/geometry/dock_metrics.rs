/// Numeric layout constants passed from `app` into the platform-independent core.
/// All dimensions are logical pixels (`f32`), not physical display pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockMetrics {
    pub column_header_height: f32,
    pub tab_strip_height: f32,
    pub splitter_thickness: f32,
    pub splitter_hit_zone: f32,
    pub default_column_width: f32,
    pub min_column_width: f32,
    pub rail_width: f32,
    pub min_group_height: f32,
    pub collapsed_group_height: f32,
    pub min_document_width: f32,
    pub max_columns: usize,
}

impl DockMetrics {
    /// Reject unusable metric bundles before geometry calculations.
    pub fn validate(&self) -> Result<(), &'static str> {
        let dimensions = [
            self.column_header_height,
            self.tab_strip_height,
            self.splitter_thickness,
            self.splitter_hit_zone,
            self.default_column_width,
            self.min_column_width,
            self.rail_width,
            self.min_group_height,
            self.collapsed_group_height,
            self.min_document_width,
        ];
        if dimensions.iter().any(|value| !value.is_finite()) {
            return Err("metrics must be finite");
        }
        if dimensions.iter().any(|value| *value <= 0.0) {
            return Err("metrics must be positive");
        }
        if self.min_column_width > self.default_column_width {
            return Err("minimum column width exceeds default column width");
        }
        if self.splitter_hit_zone < self.splitter_thickness {
            return Err("splitter hit-zone must not be thinner than the splitter");
        }
        if self.max_columns == 0 {
            return Err("maximum column count must be greater than zero");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DockMetrics;

    #[test]
    fn rejects_non_finite_values() {
        let metrics = DockMetrics {
            column_header_height: f32::NAN,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        };
        assert!(metrics.validate().is_err());
    }
}
