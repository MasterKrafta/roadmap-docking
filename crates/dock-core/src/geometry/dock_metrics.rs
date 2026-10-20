#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockMetrics {
    pub column_header_height: f32,
    pub tab_strip_height: f32,
    pub splitter_thickness: f32,
    pub splitter_hit_zone: f32,
    pub edge_drop_zone_width: f32,
    /// Logical-pixel hit-zone width for group and column edges.
    pub drop_edge_zone: f32,
    pub drag_start_threshold: f32,
    pub default_column_width: f32,
    pub min_column_width: f32,
    pub rail_width: f32,
    pub min_group_height: f32,
    pub collapsed_group_height: f32,
    pub min_document_width: f32,
    pub max_columns: usize,
}
impl DockMetrics {
    pub fn validate(&self) -> Result<(), &'static str> {
        let values = [
            self.column_header_height,
            self.tab_strip_height,
            self.splitter_thickness,
            self.splitter_hit_zone,
            self.edge_drop_zone_width,
            self.drop_edge_zone,
            self.drag_start_threshold,
            self.default_column_width,
            self.min_column_width,
            self.rail_width,
            self.min_group_height,
            self.collapsed_group_height,
            self.min_document_width,
        ];
        if values.iter().any(|v| !v.is_finite()) {
            return Err("metrics must be finite");
        }
        if values.iter().any(|v| *v <= 0.0) {
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
        let m = DockMetrics {
            column_header_height: f32::NAN,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drop_edge_zone: 24.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        };
        assert!(m.validate().is_err());
    }
    #[test]
    fn accepts_complete_positive_metric_bundle() {
        let m = DockMetrics {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drop_edge_zone: 24.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        };
        assert!(m.validate().is_ok());
    }
}
