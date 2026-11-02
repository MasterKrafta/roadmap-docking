use dock_core::{ColumnMode, DockCommand, DockMetrics, NodeId};
use gpui::{
    Entity, IntoElement, ParentElement, StatefulInteractiveElement, Styled, div,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::DockController;

/// Fixed, non-scrolling header that belongs to the Column rather than to a Group.
pub struct ColumnHeaderView;

impl ColumnHeaderView {
    pub(crate) fn render(
        column: NodeId,
        mode: ColumnMode,
        width: f32,
        metrics: DockMetrics,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let control_width = metrics
            .group_control_button_width
            .min(width / 3.0)
            .max(1.0);
        let header_height = metrics.column_header_height;
        let next_mode = next_column_mode(mode);
        let mode_label = column_mode_label(mode);

        let mode_controller = controller;
        let mode_button = div()
            .id(format!("column-mode-{:?}", column))
            .w(px(control_width))
            .h(px(header_height))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.column_mode_button))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .child(mode_label)
            .on_click(move |_, _, app| {
                mode_controller.update(app, |controller, controller_cx| {
                    if let Err(error) = controller.dispatch_in(
                        DockCommand::SetColumnMode {
                            column,
                            mode: next_mode,
                        },
                        controller_cx,
                    ) {
                        tracing::warn!(%error, "could not change dock column mode");
                    }
                });
            });

        div()
            .id(format!("column-header-{:?}", column))
            .absolute()
            .left_0()
            .top_0()
            .w(px(width))
            .h(px(header_height))
            .flex()
            .items_center()
            .bg(rgb(theme.colors.group_frame_background))
            .child(
                div()
                    .id(format!("column-drag-zone-{:?}", column))
                    .flex_1()
                    .h_full()
                    .bg(rgb(theme.colors.group_grip_background)),
            )
            .child(mode_button)
    }
}

fn next_column_mode(mode: ColumnMode) -> ColumnMode {
    match mode {
        ColumnMode::Expanded => ColumnMode::IconsAndLabels,
        ColumnMode::IconsAndLabels => ColumnMode::IconsOnly,
        ColumnMode::IconsOnly => ColumnMode::Expanded,
    }
}

fn column_mode_label(mode: ColumnMode) -> &'static str {
    match mode {
        ColumnMode::Expanded => "⟨⟨",
        ColumnMode::IconsAndLabels => "⟨",
        ColumnMode::IconsOnly => "⟩⟩",
    }
}

#[cfg(test)]
mod tests {
    use dock_core::ColumnMode;

    use super::next_column_mode;

    #[test]
    fn column_mode_button_cycles_through_all_three_modes() {
        assert_eq!(
            next_column_mode(ColumnMode::Expanded),
            ColumnMode::IconsAndLabels
        );
        assert_eq!(
            next_column_mode(ColumnMode::IconsAndLabels),
            ColumnMode::IconsOnly
        );
        assert_eq!(
            next_column_mode(ColumnMode::IconsOnly),
            ColumnMode::Expanded
        );
    }
}
