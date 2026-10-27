#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeMetrics {
    pub column_header_height: f32,
    pub tab_strip_height: f32,
    /// Width of the collapse/close/menu buttons in a Tabs group.
    pub group_control_button_width: f32,
    /// Reserved compatibility metric for a possible future tab-strip drag target.
    /// The current UI does not consume this field.
    pub tab_drag_zone_width: f32,
    /// Maximum width of icon buttons in floating-window titles, in logical pixels.
    pub button_size: f32,
    /// Spacing between controls in floating-window titles, in logical pixels.
    pub button_gap: f32,
    /// Uniform padding applied inside each panel's content surface, in logical pixels.
    pub content_padding: f32,
    pub splitter_thickness: f32,
    pub splitter_hit_zone: f32,
    /// Legacy compatibility metric. Dock hit-zone geometry uses `drop_edge_zone`;
    /// changing this field alone does not affect the current UI.
    pub edge_drop_zone_width: f32,
    /// Logical-pixel hit-zone width for group and column docking edges.
    pub drop_edge_zone: f32,
    /// Thickness of the live drop insertion and split indicators in logical pixels.
    pub drop_indicator_thickness: f32,
    /// Thickness of the outline around a target group or floating-window preview.
    pub drop_outline_thickness: f32,
    pub drag_start_threshold: f32,
    pub default_column_width: f32,
    pub min_column_width: f32,
    /// Initial fraction of the column's content area allocated to the upper panel.
    pub default_column_split_ratio: f32,
    /// Legacy compatibility metric for the former simple-column tab UI.
    /// Current tabs size themselves from the available strip width and tab count;
    /// changing this field alone does not affect the current UI.
    pub column_tab_width: f32,
    pub rail_width: f32,
    /// Width at which resizing an expanded column automatically enters icon-rail mode.
    pub rail_collapse_width: f32,
    /// Width at which dragging an icon rail's outer edge restores expanded mode.
    pub rail_expand_width: f32,
    pub min_group_height: f32,
    pub collapsed_group_height: f32,
    pub min_document_width: f32,
    pub max_columns: usize,
}

impl ThemeMetrics {
    /// Validates finite, positive layout metrics and their required relationships.
    pub fn validate(self) -> Result<(), &'static str> {
        let positive = [
            ("column_header_height", self.column_header_height),
            ("tab_strip_height", self.tab_strip_height),
            ("group_control_button_width", self.group_control_button_width),
            ("tab_drag_zone_width", self.tab_drag_zone_width),
            ("button_size", self.button_size),
            ("splitter_thickness", self.splitter_thickness),
            ("splitter_hit_zone", self.splitter_hit_zone),
            ("edge_drop_zone_width", self.edge_drop_zone_width),
            ("drop_edge_zone", self.drop_edge_zone),
            ("drop_indicator_thickness", self.drop_indicator_thickness),
            ("drop_outline_thickness", self.drop_outline_thickness),
            ("drag_start_threshold", self.drag_start_threshold),
            ("default_column_width", self.default_column_width),
            ("min_column_width", self.min_column_width),
            ("column_tab_width", self.column_tab_width),
            ("rail_width", self.rail_width),
            ("rail_collapse_width", self.rail_collapse_width),
            ("rail_expand_width", self.rail_expand_width),
            ("min_group_height", self.min_group_height),
            ("collapsed_group_height", self.collapsed_group_height),
            ("min_document_width", self.min_document_width),
        ];

        for (name, value) in positive {
            if !value.is_finite() {
                return Err(match name {
                    "column_header_height" => "column_header_height must be finite",
                    "tab_strip_height" => "tab_strip_height must be finite",
                    "group_control_button_width" => "group_control_button_width must be finite",
                    "tab_drag_zone_width" => "tab_drag_zone_width must be finite",
                    "button_size" => "button_size must be finite",
                    "splitter_thickness" => "splitter_thickness must be finite",
                    "splitter_hit_zone" => "splitter_hit_zone must be finite",
                    "edge_drop_zone_width" => "edge_drop_zone_width must be finite",
                    "drop_edge_zone" => "drop_edge_zone must be finite",
                    "drop_indicator_thickness" => "drop_indicator_thickness must be finite",
                    "drop_outline_thickness" => "drop_outline_thickness must be finite",
                    "drag_start_threshold" => "drag_start_threshold must be finite",
                    "default_column_width" => "default_column_width must be finite",
                    "min_column_width" => "min_column_width must be finite",
                    "column_tab_width" => "column_tab_width must be finite",
                    "rail_width" => "rail_width must be finite",
                    "rail_collapse_width" => "rail_collapse_width must be finite",
                    "rail_expand_width" => "rail_expand_width must be finite",
                    "min_group_height" => "min_group_height must be finite",
                    "collapsed_group_height" => "collapsed_group_height must be finite",
                    "min_document_width" => "min_document_width must be finite",
                    _ => "theme metric must be finite",
                });
            }
            if value <= 0.0 {
                return Err(match name {
                    "column_header_height" => "column_header_height must be positive",
                    "tab_strip_height" => "tab_strip_height must be positive",
                    "group_control_button_width" => "group_control_button_width must be positive",
                    "tab_drag_zone_width" => "tab_drag_zone_width must be positive",
                    "button_size" => "button_size must be positive",
                    "splitter_thickness" => "splitter_thickness must be positive",
                    "splitter_hit_zone" => "splitter_hit_zone must be positive",
                    "edge_drop_zone_width" => "edge_drop_zone_width must be positive",
                    "drop_edge_zone" => "drop_edge_zone must be positive",
                    "drop_indicator_thickness" => "drop_indicator_thickness must be positive",
                    "drop_outline_thickness" => "drop_outline_thickness must be positive",
                    "drag_start_threshold" => "drag_start_threshold must be positive",
                    "default_column_width" => "default_column_width must be positive",
                    "min_column_width" => "min_column_width must be positive",
                    "column_tab_width" => "column_tab_width must be positive",
                    "rail_width" => "rail_width must be positive",
                    "rail_collapse_width" => "rail_collapse_width must be positive",
                    "rail_expand_width" => "rail_expand_width must be positive",
                    "min_group_height" => "min_group_height must be positive",
                    "collapsed_group_height" => "collapsed_group_height must be positive",
                    "min_document_width" => "min_document_width must be positive",
                    _ => "theme metric must be positive",
                });
            }
        }

        for (name, value) in [
            ("button_gap", self.button_gap),
            ("content_padding", self.content_padding),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(if name == "button_gap" {
                    "button_gap must be finite and non-negative"
                } else {
                    "content_padding must be finite and non-negative"
                });
            }
        }

        if !self.default_column_split_ratio.is_finite()
            || !(0.0..=1.0).contains(&self.default_column_split_ratio)
            || self.default_column_split_ratio == 0.0
            || self.default_column_split_ratio == 1.0
        {
            return Err("default_column_split_ratio must be finite and strictly between 0 and 1");
        }
        if self.min_column_width > self.default_column_width {
            return Err("min_column_width must not exceed default_column_width");
        }
        if self.rail_width > self.min_column_width {
            return Err("rail_width must not exceed min_column_width");
        }
        if self.splitter_thickness > self.splitter_hit_zone {
            return Err("splitter_thickness must not exceed splitter_hit_zone");
        }
        if self.rail_expand_width <= self.rail_collapse_width {
            return Err("rail_expand_width must exceed rail_collapse_width");
        }
        if self.collapsed_group_height > self.min_group_height {
            return Err("collapsed_group_height must not exceed min_group_height");
        }
        if self.column_header_height + self.tab_strip_height > self.min_group_height {
            return Err("column_header_height + tab_strip_height must not exceed min_group_height");
        }
        if self.max_columns == 0 {
            return Err("max_columns must be positive");
        }

        Ok(())
    }
}

impl Default for ThemeMetrics {
    fn default() -> Self {
        Self {
            column_header_height: 32.0,
            tab_strip_height: 28.0,
            group_control_button_width: 34.0,
            tab_drag_zone_width: 28.0,
            button_size: 24.0,
            button_gap: 4.0,
            content_padding: 8.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_zone_width: 24.0,
            drop_edge_zone: 24.0,
            drop_indicator_thickness: 4.0,
            drop_outline_thickness: 3.0,
            drag_start_threshold: 4.0,
            default_column_width: 280.0,
            min_column_width: 200.0,
            default_column_split_ratio: 0.5,
            column_tab_width: 104.0,
            rail_width: 36.0,
            rail_collapse_width: 220.0,
            rail_expand_width: 240.0,
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
        let m = ThemeMetrics::default();
        assert!((m.default_column_width - 280.0).abs() < 1e-3);
        assert!((m.min_column_width - 200.0).abs() < 1e-3);
        assert!((m.rail_width - 36.0).abs() < 1e-3);
        assert!((m.rail_collapse_width - 220.0).abs() < 1e-3);
        assert!((m.rail_expand_width - 240.0).abs() < 1e-3);
        assert_eq!(m.max_columns, 4);
    }

    #[test]
    fn includes_drag_and_drop_threshold_metrics() {
        let m = ThemeMetrics::default();
        assert!((m.edge_drop_zone_width - 24.0).abs() < 1e-3);
        assert!((m.drop_edge_zone - 24.0).abs() < 1e-3);
        assert!((m.drag_start_threshold - 4.0).abs() < 1e-3);
        assert!((m.drop_indicator_thickness - 4.0).abs() < 1e-3);
        assert!((m.drop_outline_thickness - 3.0).abs() < 1e-3);
    }

    #[test]
    fn includes_tabs_group_chrome_metrics() {
        let m = ThemeMetrics::default();
        assert!((m.group_control_button_width - 34.0).abs() < 1e-3);
        assert!((m.tab_drag_zone_width - 28.0).abs() < 1e-3);
    }

    #[test]
    fn includes_main_window_column_metrics() {
        let m = ThemeMetrics::default();
        assert!((m.column_header_height - 32.0).abs() < 1e-3);
        assert!((m.default_column_width - 280.0).abs() < 1e-3);
        assert!((m.min_column_width - 200.0).abs() < 1e-3);
        assert!((m.default_column_split_ratio - 0.5).abs() < 1e-3);
        assert!((m.column_tab_width - 104.0).abs() < 1e-3);
        assert!((m.splitter_thickness - 4.0).abs() < 1e-3);
        assert!((m.splitter_hit_zone - 8.0).abs() < 1e-3);
    }
}
