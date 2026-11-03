#[path = "column_drag_area.rs"]
mod column_drag_area;
#[path = "collapse_column_button.rs"]
mod collapse_column_button;
#[path = "close_column_button.rs"]
mod close_column_button;
#[path = "group_drag_area.rs"]
mod group_drag_area;
#[path = "group_grip.rs"]
mod group_grip;
#[path = "group_menu_button.rs"]
mod group_menu_button;
#[path = "panel_content.rs"]
mod panel_content;
#[path = "tab.rs"]
mod tab;

use dock_core::DockMetrics;
use gpui::{AnyView, IntoElement, InteractiveElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

pub use column_drag_area::ColumnDragArea;
pub use collapse_column_button::CollapseColumnButton;
pub use close_column_button::CloseColumnButton;
pub use group_drag_area::GroupDragArea;
pub use group_grip::GroupGrip;
pub use group_menu_button::GroupMenuButton;
pub use panel_content::PanelContent;
pub use tab::DockTab;

/// Extra controls are retained in code but disabled in the live composition.
const SHOW_COLUMN_CONTROLS: bool = false;
const SHOW_GROUP_CONTROLS: bool = false;

/// Static visual variants for the new Column composition.
pub struct DockColumnView;

impl DockColumnView {
    /// Renders one Group occupying all available space below the Column header.
    pub fn one_group(theme: Theme) -> impl IntoElement {
        Self::render_groups("column-one-group", 1, theme)
    }

    /// Renders two Groups stacked vertically with equal available height.
    pub fn two_groups(theme: Theme) -> impl IntoElement {
        Self::render_groups("column-two-groups", 2, theme)
    }

    /// Renders a live Column with exactly one active tab and its panel view.
    ///
    /// Core still owns the Column/Group/Panel nodes; this function only composes
    /// their visual surface inside the rectangle assigned by the main-dock adapter.
    pub fn render_active(
        id: String,
        title: String,
        panel_view: AnyView,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        let group_id = format!("{id}-group");
        let group_header_height = metrics.header_height.max(metrics.tab_strip_height);

        let mut column_header = div()
            .id(format!("{id}-header"))
            .w_full()
            .h(px(metrics.column_header_height))
            .flex_none()
            .flex()
            .items_stretch()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(rgb(theme.colors.group_grip_background)),
            );

        // These controls remain available in the composition, but stay disabled
        // until their actions are connected to the DockController.
        if SHOW_COLUMN_CONTROLS {
            column_header = column_header
                .child(CollapseColumnButton::render(
                    format!("{id}-collapse"),
                    theme,
                ))
                .child(CloseColumnButton::render(format!("{id}-close"), theme));
        }

        let mut group_header = div()
            .id(format!("{group_id}-header"))
            .w_full()
            .h(px(group_header_height))
            .flex_none()
            .flex()
            .items_stretch()
            .child(DockTab::render_with_width(
                format!("{group_id}-active-tab"),
                title,
                true,
                theme,
                metrics.min_tab_width,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(rgb(theme.colors.group_grip_background)),
            );

        // Group grip and menu are intentionally not mounted yet.
        if SHOW_GROUP_CONTROLS {
            group_header = group_header
                .child(GroupGrip::render(format!("{group_id}-grip"), theme))
                .child(GroupMenuButton::render(format!("{group_id}-menu"), theme));
        }

        div()
            .id(id)
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(theme.colors.dock_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(column_header)
            .child(
                div()
                    .id(group_id)
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(theme.colors.group_frame_border))
                    .child(group_header)
                    .child(
                        div()
                            .id(format!("{group_id}-content"))
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .bg(rgb(theme.colors.panel_surface_background))
                            .child(panel_view),
                    ),
            )
    }

    fn render_groups(
        id: &'static str,
        group_count: usize,
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
            .child(
                div()
                    .id(format!("{id}-header"))
                    .w_full()
                    .h(px(32.0))
                    .flex_none()
                    .flex()
                    .items_stretch()
                    .child(ColumnDragArea::render(format!("{id}-column-drag"), theme))
                    .child(CollapseColumnButton::render(
                        format!("{id}-collapse"),
                        theme,
                    ))
                    .child(CloseColumnButton::render(format!("{id}-close"), theme)),
            );

        for index in 0..group_count {
            let group_id = format!("{id}-group-{index}");
            let group = div()
                .id(group_id.clone())
                .w_full()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_hidden()
                .border_1()
                .border_color(rgb(theme.colors.group_frame_border))
                .child(
                    div()
                        .id(format!("{group_id}-header"))
                        .w_full()
                        .h(px(32.0))
                        .flex_none()
                        .flex()
                        .items_stretch()
                        .child(DockTab::render(
                            format!("{group_id}-tab-one"),
                            "Вкладка 1".to_owned(),
                            true,
                            theme,
                        ))
                        .child(GroupDragArea::render(
                            format!("{group_id}-drag"),
                            theme,
                        ))
                        .child(GroupGrip::render(format!("{group_id}-grip"), theme))
                        .child(GroupMenuButton::render(format!("{group_id}-menu"), theme)),
                )
                .child(PanelContent::render(
                    format!("{group_id}-content"),
                    theme,
                ));
            column = column.child(group);
        }

        column
    }
}
