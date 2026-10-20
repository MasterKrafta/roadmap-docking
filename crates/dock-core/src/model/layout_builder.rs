//! Test-only builder for compact, readable docking compositions.
//!
//! Example:
//! let mut builder = LayoutBuilder::new(metrics);
//! builder.dock_right().column(280.0);
//! builder.group(0.5, ["layers", "color"])?;
//! builder.group(0.5, ["properties", "history"])?;

use crate::{
    CanvasState, Column, ColumnMode, DockArea, DockMetrics, DockSide, Layout, Node,
    PanelConstraints, PanelInstance, PanelKey, Profile, Size, Tabs,
};

pub(super) struct LayoutBuilder {
    layout: Layout,
    metrics: DockMetrics,
    side: DockSide,
    columns: Vec<Column>,
    current_column: Option<Column>,
}

impl LayoutBuilder {
    pub(super) fn new(metrics: DockMetrics) -> Self {
        Self {
            layout: Layout::new(Profile::Photoshop, CanvasState::default()),
            metrics,
            side: DockSide::Right,
            columns: Vec::new(),
            current_column: None,
        }
    }

    pub(super) fn dock_right(&mut self) -> &mut Self {
        self.side = DockSide::Right;
        self
    }

    pub(super) fn dock_left(&mut self) -> &mut Self {
        self.side = DockSide::Left;
        self
    }

    pub(super) fn column(&mut self, width: f32) -> &mut Self {
        self.finish_current_column();
        self.current_column = Some(Column {
            width,
            mode: ColumnMode::Expanded,
            groups: Vec::new(),
            group_ratios: Vec::new(),
        });
        self
    }

    pub(super) fn group<I, S>(
        &mut self,
        ratio: f32,
        panel_keys: I,
    ) -> Result<&mut Self, &'static str>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        if self.current_column.is_none() {
            return Err("call column() before group()");
        }
        if !ratio.is_finite() || ratio < 0.0 {
            return Err("group ratio must be finite and non-negative");
        }

        let keys: Vec<PanelKey> = panel_keys
            .into_iter()
            .map(|key| PanelKey::new(key.as_ref()))
            .collect::<Result<_, _>>()?;
        if keys.is_empty() {
            return Err("a test group must contain at least one panel");
        }

        let constraints = PanelConstraints::new(
            Size::new(
                self.metrics.min_column_width,
                self.metrics.min_group_height,
            ),
            Size::new(
                self.metrics.default_column_width,
                self.metrics.min_group_height,
            ),
            Size::new(f32::MAX, f32::MAX),
            true,
        );

        let panel_ids = keys
            .into_iter()
            .map(|key| self.layout.insert_panel(PanelInstance::new(key, constraints)))
            .collect();
        let group_id = self.layout.insert_node(Node::Tabs(Tabs {
            panels: panel_ids,
            active_index: 0,
        }));

        let column = self
            .current_column
            .as_mut()
            .ok_or("call column() before group()")?;
        column.groups.push(group_id);
        column.group_ratios.push(ratio);
        Ok(self)
    }

    pub(super) fn build(mut self) -> Result<Layout, &'static str> {
        self.finish_current_column();
        if self.columns.is_empty() {
            return Err("a test layout must contain at least one column");
        }

        self.layout.push_dock_area(DockArea {
            side: self.side,
            columns: self.columns,
        });
        self.layout
            .validate(&self.metrics)
            .map_err(|_| "test builder produced an invalid layout")?;
        Ok(self.layout)
    }

    fn finish_current_column(&mut self) {
        if let Some(column) = self.current_column.take() {
            self.columns.push(column);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LayoutBuilder;
    use crate::{DockMetrics, DockSide};
    use proptest::prelude::*;

    fn metrics() -> DockMetrics {
        DockMetrics {
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
        }
    }

    #[test]
    fn compact_builder_creates_right_dock_and_ordered_tab_groups() {
        let metrics = metrics();
        let mut builder = LayoutBuilder::new(metrics);
        builder.dock_right().column(280.0);
        builder
            .group(0.5, ["layers", "color"])
            .expect("valid group");
        builder
            .group(0.5, ["properties", "history"])
            .expect("valid group");

        let layout = builder.build().expect("one column was built");
        assert_eq!(layout.dock_areas().len(), 1);
        assert_eq!(layout.dock_areas()[0].side, DockSide::Right);
        assert_eq!(layout.dock_areas()[0].columns.len(), 1);
        assert_eq!(layout.dock_areas()[0].columns[0].groups.len(), 2);
        assert_eq!(layout.panels().len(), 4);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn builder_supports_a_single_group_column_and_left_dock() {
        let metrics = metrics();
        let mut builder = LayoutBuilder::new(metrics);
        builder.dock_left().column(280.0);
        builder.group(1.0, ["layers"]).expect("valid single group");

        let layout = builder.build().expect("one column was built");
        assert_eq!(layout.dock_areas()[0].side, DockSide::Left);
        assert_eq!(layout.dock_areas()[0].columns[0].groups.len(), 1);
        assert!(layout.validate(&metrics).is_ok());
    }

    #[test]
    fn builder_rejects_groups_without_a_column_and_empty_groups() {
        let metrics = metrics();
        let mut builder = LayoutBuilder::new(metrics);
        assert!(builder.group(1.0, ["layers"]).is_err());

        builder.column(280.0);
        assert!(builder.group(1.0, std::iter::empty::<&str>()).is_err());
        assert!(builder.group(1.0, [""]).is_err());
        assert!(builder.group(f32::NAN, ["layers"]).is_err());
    }

    #[test]
    fn builder_rejects_empty_columns_when_building() {
        let mut builder = LayoutBuilder::new(metrics());
        builder.column(280.0);
        assert!(builder.build().is_err());
    }

    proptest! {
        #[test]
        fn generated_photoshop_compositions_always_validate(
            group_shapes in prop::collection::vec(
                prop::collection::vec(1usize..=4, 1..=4),
                1..=4,
            )
        ) {
            let metrics = metrics();
            let mut builder = LayoutBuilder::new(metrics);
            builder.dock_right();

            let expected_panel_count: usize = group_shapes
                .iter()
                .flat_map(|groups| groups.iter())
                .sum();

            for (column_index, group_panel_counts) in group_shapes.iter().enumerate() {
                builder.column(metrics.default_column_width);
                let ratio = 1.0 / group_panel_counts.len() as f32;

                for (group_index, panel_count) in group_panel_counts.iter().copied().enumerate() {
                    let keys: Vec<String> = (0..panel_count)
                        .map(|panel_index| {
                            format!("panel_{column_index}_{group_index}_{panel_index}")
                        })
                        .collect();
                    builder
                        .group(ratio, keys)
                        .expect("generated panel keys and group ratios are valid");
                }
            }

            let layout = builder.build().expect("generator always creates columns");
            prop_assert!(
                layout.validate(&metrics).is_ok(),
                "generated Photoshop composition must validate"
            );
            prop_assert_eq!(layout.panels().len(), expected_panel_count);
            prop_assert!(
                layout.nodes().values().all(|node| matches!(node, crate::Node::Tabs(_))),
                "Photoshop column groups must be tab nodes"
            );
        }
    }
}
