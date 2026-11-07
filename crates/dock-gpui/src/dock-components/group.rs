#[path = "tab.rs"]
mod tab;
#[path = "group_grip.rs"]
mod group_grip;
#[path = "group_drag_area.rs"]
mod group_drag_area;
#[path = "group_menu_button.rs"]
mod group_menu_button;
#[path = "panel_content.rs"]
mod panel_content;

pub use tab::DockTab;
pub use group_grip::GroupGrip;
pub use group_drag_area::GroupDragArea;
pub use group_menu_button::GroupMenuButton;
pub use panel_content::PanelContent;
use dock_core::DockMetrics;
use gpui::{AnyView, InteractiveElement, IntoElement, ParentElement, Styled, div, px, rgb};
use ui::Theme;

const SHOW_GROUP_CONTROLS: bool = true;

/// Renders one Group inside a Column.
pub(super) struct DockGroupView;

impl DockGroupView {
    /// Renders a live Group with one active tab and its panel view.
    pub(super) fn render_active(
        id: String,
        title: String,
        panel_view: AnyView,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        let header = Self::render_header(
            format!("{id}-header"),
            format!("{id}-active-tab"),
            title,
            metrics.min_tab_width,
            metrics,
            theme,
        );

        div()
            .id(id.clone())
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(header)
            .child(
                div()
                    .id(format!("{id}-content"))
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .bg(rgb(theme.colors.panel_surface_background))
                    .child(panel_view),
            )
    }

    /// Renders a placeholder Group used by static Column previews.
    pub(super) fn render_preview(
        id: String,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        let header = Self::render_header(
            format!("{id}-header"),
            format!("{id}-tab-one"),
            "Вкладка 1".to_owned(),
            metrics.default_tab_width,
            metrics,
            theme,
        );

        div()
            .id(id.clone())
            .w_full()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .child(header)
            .child(PanelContent::render(format!("{id}-content"), theme))
    }

    fn render_header(
        id: String,
        tab_id: String,
        title: String,
        tab_width: f32,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        let group_header_height = metrics.header_height.max(metrics.tab_strip_height);

        let mut header = div()
            .id(id.clone())
            .w_full()
            .h(px(group_header_height))
            .flex_none()
            .flex()
            .items_stretch()
            .child(DockTab::render_with_metrics(
                tab_id,
                title,
                true,
                theme,
                tab_width,
                metrics.min_tab_width,
                metrics.tab_horizontal_padding,
            ))
            .child(GroupDragArea::render(format!("{id}-drag-area"), theme));

        // Group grip and menu are visual only; their actions are not connected yet.
        if SHOW_GROUP_CONTROLS {
            header = header
                .child(GroupGrip::render_with_metrics(
                    format!("{id}-grip"),
                    theme,
                    metrics.group_grip_width,
                    metrics.control_icon_size,
                ))
                .child(GroupMenuButton::render_with_metrics(
                    format!("{id}-menu"),
                    theme,
                    metrics.group_control_button_width,
                    metrics.control_icon_size,
                ));
        }

        header
    }
}
