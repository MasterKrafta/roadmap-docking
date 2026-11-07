#[path = "column.rs"]
mod column;
#[path = "column_drag_area.rs"]
mod column_drag_area;
#[path = "column_header.rs"]
mod column_header;
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

use gpui::{AnyView, InteractiveElement, IntoElement, ParentElement, Styled, div};
use ui::Theme;

pub use column::DockColumnView;
pub use column_drag_area::ColumnDragArea;
pub use column_header::ColumnHeader;
pub use collapse_column_button::CollapseColumnButton;
pub use close_column_button::CloseColumnButton;
pub use group_drag_area::GroupDragArea;
pub use group_grip::GroupGrip;
pub use group_menu_button::GroupMenuButton;
pub use panel_content::PanelContent;
pub use tab::DockTab;

/// Root presentation for one or more Core Columns.
///
/// Core owns the horizontal split tree and supplies each Column's rectangle.
/// This element is the shared parent for all those Column renderers.
pub struct GroupColumnsView;

impl GroupColumnsView {
    pub(crate) fn render_root(
    ) -> impl IntoElement + InteractiveElement + ParentElement + Styled {
        div()
            .id("dock-group-columns")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
    }

    /// Renders one Column from the Core-provided geometry snapshot.
    pub(crate) fn render_active_column(
        id: String,
        title: String,
        panel_view: AnyView,
        metrics: dock_core::DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        DockColumnView::render_active(id, title, panel_view, metrics, theme)
    }
}
