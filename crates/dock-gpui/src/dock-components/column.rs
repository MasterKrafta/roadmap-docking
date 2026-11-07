#[path = "group.rs"]
mod group;

use dock_core::DockMetrics;
use gpui::{AnyView, InteractiveElement, IntoElement, ParentElement, Styled, div, rgb};
use group::DockGroupView;
use super::ColumnHeader;
use ui::Theme;

/// Renders one Column containing one or more vertically stacked Groups.
pub struct DockColumnView;

impl DockColumnView {
    /// Renders one Group occupying all available space below the Column header.
    pub fn one_group(theme: Theme) -> impl IntoElement {
        Self::render_groups("column-one-group", 1, DockMetrics::default(), theme)
    }

    /// Renders two Groups stacked vertically with equal available height.
    pub fn two_groups(theme: Theme) -> impl IntoElement {
        Self::render_groups("column-two-groups", 2, DockMetrics::default(), theme)
    }

    /// Renders a live Column with exactly one active tab and its panel view.
    ///
    /// Core still owns the Column/Group/Panel nodes; this function composes
    /// their visual surface inside the rectangle assigned by the main-dock adapter.
    pub fn render_active(
        id: String,
        title: String,
        panel_view: AnyView,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        Self::render_active_with_header(id, title, panel_view, metrics, theme, true)
    }

    /// Hides the ColumnHeader during a DockTab-origin tear-off without
    /// changing the Group's layout position. The header slot remains reserved,
    /// so showing the header after release does not push the tab row downward.
    pub(crate) fn render_active_with_header(
        id: String,
        title: String,
        panel_view: AnyView,
        metrics: DockMetrics,
        theme: Theme,
        show_column_header: bool,
    ) -> impl IntoElement {
        let group_id = format!("{id}-group");
        let mut column = div()
            .id(id.clone())
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border));

        if show_column_header {
            column = column.child(ColumnHeader::render(id.clone(), metrics, theme));
        } else {
            column = column.child(
                div()
                    .id(format!("{id}-header-placeholder"))
                    .w_full()
                    .h(gpui::px(metrics.column_header_height))
                    .flex_none(),
            );
        }

        column.child(DockGroupView::render_active(
            group_id, title, panel_view, metrics, theme,
        ))
    }

    fn render_groups(
        id: &'static str,
        group_count: usize,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        let mut column = div()
            .id(id)
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(ColumnHeader::render(id.to_owned(), metrics, theme));

        for index in 0..group_count {
            let group_id = format!("{id}-group-{index}");
            column = column.child(DockGroupView::render_preview(group_id, metrics, theme));
        }

        column
    }
}
