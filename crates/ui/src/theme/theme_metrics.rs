/// Shared UI dimensions. Values are logical pixels, independent of display DPI.
/// Several values are provisional until they are checked against Photoshop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeMetrics {
    pub column_header_height: f32,
    pub tab_strip_height: f32,
    pub button_size: f32,
    pub button_gap: f32,
    pub content_padding: f32,
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

impl Default for ThemeMetrics {
    fn default() -> Self {
        Self {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            button_size: 24.0,
            button_gap: 4.0,
            content_padding: 8.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            min_document_width: 320.0,
            max_columns: 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThemeMetrics;

    #[test]
    fn preserves_initial_metrics_from_the_roadmap_fragment() {
        let metrics = ThemeMetrics::default();
        assert_eq!(metrics.default_column_width, 280.0);
        assert_eq!(metrics.min_column_width, 200.0);
        assert_eq!(metrics.rail_width, 36.0);
        assert_eq!(metrics.max_columns, 4);
    }
}
