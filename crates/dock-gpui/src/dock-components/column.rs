#[path = "column_drag_area.rs"]
mod column_drag_area;
#[path = "collapse_column_button.rs"]
mod collapse_column_button;
#[path = "close_column_button.rs"]
mod close_column_button;
#[path = "first_tab.rs"]
mod first_tab;
#[path = "group_drag_area.rs"]
mod group_drag_area;
#[path = "group_grip.rs"]
mod group_grip;
#[path = "group_menu_button.rs"]
mod group_menu_button;
#[path = "panel_content.rs"]
mod panel_content;
#[path = "second_tab.rs"]
mod second_tab;

use gpui::{IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

pub use column_drag_area::ColumnDragArea;
pub use collapse_column_button::CollapseColumnButton;
pub use close_column_button::CloseColumnButton;
pub use first_tab::FirstTab;
pub use group_drag_area::GroupDragArea;
pub use group_grip::GroupGrip;
pub use group_menu_button::GroupMenuButton;
pub use panel_content::PanelContent;
pub use second_tab::SecondTab;

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
                        .child(FirstTab::render(
                            format!("{group_id}-tab-one"),
                            "Вкладка 1".to_owned(),
                            theme,
                        ))
                        .child(SecondTab::render(
                            format!("{group_id}-tab-two"),
                            "Вкладка 2".to_owned(),
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
