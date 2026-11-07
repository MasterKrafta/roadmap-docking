#[path = "column_drag_area.rs"]
mod column_drag_area;
#[path = "column_header.rs"]
mod column_header;
#[path = "collapse_column_button.rs"]
mod collapse_column_button;
#[path = "close_column_button.rs"]
mod close_column_button;
#[path = "group_column.rs"]
mod group_column;
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
use gpui::{AnyView, InteractiveElement, IntoElement, ParentElement, Styled, div, rgb};
use group_column::GroupColumnView;
use ui::Theme;

pub use column_drag_area::ColumnDragArea;
pub use column_header::ColumnHeader;
pub use collapse_column_button::CollapseColumnButton;
pub use close_column_button::CloseColumnButton;
pub use group_drag_area::GroupDragArea;
pub use group_grip::GroupGrip;
pub use group_menu_button::GroupMenuButton;
pub use panel_content::PanelContent;
pub use tab::DockTab;

/// Static visual variants for the new Column composition.
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
    /// Core still owns the Column/Group/Panel nodes; this function only composes
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

    /// Floating tear-off uses the same Column renderer but temporarily removes
    /// ColumnHeader and its layout height while a DockTab-origin drag is held.
    /// The native window bounds are cropped by the same amount to keep the tab
    /// aligned with the original cursor position.
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
        }

        column.child(GroupColumnView::render_active(
            group_id, title, panel_view, metrics, theme,
        ))
    }

    fn render_groups(
        id: &'static str,
        group_count: usize,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        let column_header = ColumnHeader::render(id.to_owned(), metrics, theme);

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
            .child(column_header);

        for index in 0..group_count {
            let group_id = format!("{id}-group-{index}");
            column = column.child(GroupColumnView::render_preview(group_id, metrics, theme));
        }

        column
    }
}
