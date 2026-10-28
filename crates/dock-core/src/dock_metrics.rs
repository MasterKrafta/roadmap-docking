/// Central source of logical-pixel metrics used by core docking algorithms.
///
/// Values are preliminary defaults pending confirmation against the live
/// Photoshop reference/design. UI code should consume this object rather than
/// re-declaring these constants.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DockMetrics {
    pub default_column_width: f32,
    pub min_column_width: f32,
    pub rail_width: f32,
    pub splitter_thickness: f32,
    pub splitter_hit_zone: f32,
    pub edge_drop_width: f32,
    pub drag_threshold: f32,
    pub min_group_height: f32,
    pub collapsed_group_height: f32,
    pub header_height: f32,
    pub tab_strip_height: f32,
    /// Width of group controls such as the mode switch and group menu.
    pub group_control_button_width: f32,
    /// Minimum width retained by each visible tab before overflow is used.
    pub min_tab_width: f32,
    /// Width reserved for the implementation-specific hidden-tabs menu button.
    pub tab_overflow_button_width: f32,
    pub min_canvas_width: f32,
    pub max_columns: usize,
}

impl DockMetrics {
    /// Height occupied by a collapsed group, never smaller than its tab strip.
    pub(crate) const fn effective_collapsed_group_height(self) -> f32 {
        if self.collapsed_group_height > self.tab_strip_height {
            self.collapsed_group_height
        } else {
            self.tab_strip_height
        }
    }

    /// Validates finiteness, positivity, and the required metric relationships.
    pub fn validate(self) -> Result<(), &'static str> {
        let positive = [
            (
                "default_column_width",
                self.default_column_width,
                "default_column_width must be finite",
                "default_column_width must be positive",
            ),
            (
                "min_column_width",
                self.min_column_width,
                "min_column_width must be finite",
                "min_column_width must be positive",
            ),
            (
                "rail_width",
                self.rail_width,
                "rail_width must be finite",
                "rail_width must be positive",
            ),
            (
                "splitter_thickness",
                self.splitter_thickness,
                "splitter_thickness must be finite",
                "splitter_thickness must be positive",
            ),
            (
                "splitter_hit_zone",
                self.splitter_hit_zone,
                "splitter_hit_zone must be finite",
                "splitter_hit_zone must be positive",
            ),
            (
                "edge_drop_width",
                self.edge_drop_width,
                "edge_drop_width must be finite",
                "edge_drop_width must be positive",
            ),
            (
                "drag_threshold",
                self.drag_threshold,
                "drag_threshold must be finite",
                "drag_threshold must be positive",
            ),
            (
                "min_group_height",
                self.min_group_height,
                "min_group_height must be finite",
                "min_group_height must be positive",
            ),
            (
                "collapsed_group_height",
                self.collapsed_group_height,
                "collapsed_group_height must be finite",
                "collapsed_group_height must be positive",
            ),
            (
                "header_height",
                self.header_height,
                "header_height must be finite",
                "header_height must be positive",
            ),
            (
                "tab_strip_height",
                self.tab_strip_height,
                "tab_strip_height must be finite",
                "tab_strip_height must be positive",
            ),
            (
                "group_control_button_width",
                self.group_control_button_width,
                "group_control_button_width must be finite",
                "group_control_button_width must be positive",
            ),
            (
                "min_tab_width",
                self.min_tab_width,
                "min_tab_width must be finite",
                "min_tab_width must be positive",
            ),
            (
                "tab_overflow_button_width",
                self.tab_overflow_button_width,
                "tab_overflow_button_width must be finite",
                "tab_overflow_button_width must be positive",
            ),
            (
                "min_canvas_width",
                self.min_canvas_width,
                "min_canvas_width must be finite",
                "min_canvas_width must be positive",
            ),
        ];

        for (_name, value, non_finite, non_positive) in positive {
            if !value.is_finite() {
                return Err(non_finite);
            }
            if value <= 0.0 {
                return Err(non_positive);
            }
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
        if self.collapsed_group_height > self.min_group_height {
            return Err("collapsed_group_height must not exceed min_group_height");
        }
        if self.header_height + self.tab_strip_height > self.min_group_height {
            return Err("header_height + tab_strip_height must not exceed min_group_height");
        }
        if self.max_columns == 0 {
            return Err("max_columns must be positive");
        }

        Ok(())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for DockMetrics {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(default)]
        struct Wire {
            default_column_width: f32,
            min_column_width: f32,
            rail_width: f32,
            splitter_thickness: f32,
            splitter_hit_zone: f32,
            edge_drop_width: f32,
            drag_threshold: f32,
            min_group_height: f32,
            collapsed_group_height: f32,
            header_height: f32,
            tab_strip_height: f32,
            group_control_button_width: f32,
            min_tab_width: f32,
            tab_overflow_button_width: f32,
            min_canvas_width: f32,
            max_columns: usize,
        }

        impl Default for Wire {
            fn default() -> Self {
                let metrics = DockMetrics::default();
                Self {
                    default_column_width: metrics.default_column_width,
                    min_column_width: metrics.min_column_width,
                    rail_width: metrics.rail_width,
                    splitter_thickness: metrics.splitter_thickness,
                    splitter_hit_zone: metrics.splitter_hit_zone,
                    edge_drop_width: metrics.edge_drop_width,
                    drag_threshold: metrics.drag_threshold,
                    min_group_height: metrics.min_group_height,
                    collapsed_group_height: metrics.collapsed_group_height,
                    header_height: metrics.header_height,
                    tab_strip_height: metrics.tab_strip_height,
                    group_control_button_width: metrics.group_control_button_width,
                    min_tab_width: metrics.min_tab_width,
                    tab_overflow_button_width: metrics.tab_overflow_button_width,
                    min_canvas_width: metrics.min_canvas_width,
                    max_columns: metrics.max_columns,
                }
            }
        }

        let wire = Wire::deserialize(deserializer)?;
        let metrics = Self {
            default_column_width: wire.default_column_width,
            min_column_width: wire.min_column_width,
            rail_width: wire.rail_width,
            splitter_thickness: wire.splitter_thickness,
            splitter_hit_zone: wire.splitter_hit_zone,
            edge_drop_width: wire.edge_drop_width,
            drag_threshold: wire.drag_threshold,
            min_group_height: wire.min_group_height,
            collapsed_group_height: wire.collapsed_group_height,
            header_height: wire.header_height,
            tab_strip_height: wire.tab_strip_height,
            group_control_button_width: wire.group_control_button_width,
            min_tab_width: wire.min_tab_width,
            tab_overflow_button_width: wire.tab_overflow_button_width,
            min_canvas_width: wire.min_canvas_width,
            max_columns: wire.max_columns,
        };
        metrics.validate().map_err(serde::de::Error::custom)?;
        Ok(metrics)
    }
}

impl Default for DockMetrics {
    fn default() -> Self {
        Self {
            default_column_width: 280.0,
            min_column_width: 200.0,
            rail_width: 36.0,
            splitter_thickness: 4.0,
            splitter_hit_zone: 8.0,
            edge_drop_width: 24.0,
            drag_threshold: 4.0,
            min_group_height: 80.0,
            collapsed_group_height: 28.0,
            // These two heights were not specified by the roadmap excerpt;
            // 28 px is a provisional placeholder for each.
            header_height: 28.0,
            tab_strip_height: 28.0,
            group_control_button_width: 28.0,
            min_tab_width: 72.0,
            tab_overflow_button_width: 28.0,
            min_canvas_width: 320.0,
            max_columns: 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DockMetrics;

    #[test]
    fn provisional_defaults_are_valid() {
        assert_eq!(DockMetrics::default().validate(), Ok(()));
        assert_eq!(DockMetrics::default().rail_width, 36.0);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn legacy_metrics_deserialize_with_default_tab_overflow_values() {
        let legacy = r#"{
            "default_column_width": 280.0,
            "min_column_width": 200.0,
            "rail_width": 36.0,
            "splitter_thickness": 4.0,
            "splitter_hit_zone": 8.0,
            "edge_drop_width": 24.0,
            "drag_threshold": 4.0,
            "min_group_height": 80.0,
            "collapsed_group_height": 28.0,
            "header_height": 28.0,
            "tab_strip_height": 28.0,
            "min_canvas_width": 320.0,
            "max_columns": 4
        }"#;
        let metrics: DockMetrics = serde_json::from_str(legacy).unwrap();
        assert_eq!(metrics.group_control_button_width, 28.0);
        assert_eq!(metrics.min_tab_width, 72.0);
        assert_eq!(metrics.tab_overflow_button_width, 28.0);
        assert_eq!(metrics.validate(), Ok(()));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_deserialization_rejects_invalid_metric_relationships() {
        let invalid =
            r#"{ "header_height": 60.0, "tab_strip_height": 40.0, "min_group_height": 80.0 }"#;
        assert!(serde_json::from_str::<DockMetrics>(invalid).is_err());
        let defaults: DockMetrics = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults, DockMetrics::default());
    }

    #[test]
    fn rejects_non_finite_and_non_positive_metrics() {
        let metrics = DockMetrics {
            drag_threshold: f32::NAN,
            ..DockMetrics::default()
        };
        assert!(metrics.validate().is_err());

        let metrics = DockMetrics {
            min_canvas_width: 0.0,
            ..DockMetrics::default()
        };
        assert!(metrics.validate().is_err());

        let metrics = DockMetrics {
            splitter_thickness: f32::INFINITY,
            ..DockMetrics::default()
        };
        assert!(metrics.validate().is_err());

        let metrics = DockMetrics {
            min_tab_width: f32::NAN,
            ..DockMetrics::default()
        };
        assert!(metrics.validate().is_err());

        let metrics = DockMetrics {
            tab_overflow_button_width: 0.0,
            ..DockMetrics::default()
        };
        assert!(metrics.validate().is_err());
    }

    #[test]
    fn drop_edge_width_is_independent_from_visual_and_hit_splitter_metrics() {
        let mut metrics = DockMetrics::default();
        assert_eq!(metrics.edge_drop_width, 24.0);
        assert_eq!(metrics.splitter_thickness, 4.0);
        assert_eq!(metrics.splitter_hit_zone, 8.0);

        // These dimensions have separate jobs. Changing the edge drop affordance
        // must not rewrite the visible splitter or the splitter's hit area.
        metrics.edge_drop_width = 32.0;
        assert_eq!(metrics.validate(), Ok(()));
        assert_eq!(metrics.edge_drop_width, 32.0);
        assert_eq!(metrics.splitter_thickness, 4.0);
        assert_eq!(metrics.splitter_hit_zone, 8.0);

        metrics.splitter_hit_zone = 12.0;
        assert_eq!(metrics.validate(), Ok(()));
        assert_eq!(metrics.edge_drop_width, 32.0);
        assert_eq!(metrics.splitter_thickness, 4.0);
        assert_eq!(metrics.splitter_hit_zone, 12.0);
    }

    #[test]
    fn rejects_contradictory_relationships() {
        let metrics = DockMetrics {
            min_column_width: 300.0,
            ..DockMetrics::default()
        };
        assert!(metrics.validate().is_err());

        let metrics = DockMetrics {
            collapsed_group_height: DockMetrics::default().min_group_height + 1.0,
            ..DockMetrics::default()
        };
        assert!(metrics.validate().is_err());

        let metrics = DockMetrics {
            max_columns: 0,
            ..DockMetrics::default()
        };
        assert!(metrics.validate().is_err());

        let metrics = DockMetrics {
            min_group_height: 40.0,
            ..DockMetrics::default()
        };
        assert_eq!(
            metrics.validate(),
            Err("header_height + tab_strip_height must not exceed min_group_height")
        );
    }
}
