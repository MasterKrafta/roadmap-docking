#[path = "group_grip.rs"]
mod group_grip;
#[path = "group_drag_area.rs"]
mod group_drag_area;
#[path = "group_menu_button.rs"]
mod group_menu_button;
#[path = "panel_content.rs"]
mod panel_content;

pub use group_grip::GroupGrip;
pub use group_drag_area::GroupDragArea;
pub use group_menu_button::GroupMenuButton;
pub use panel_content::PanelContent;

use dock_core::{
    DockCommand, DockMetrics, GroupRect, NodeId, Point, Rect, TabHeaderRect,
    TabOverflowRect, TabStripRect,
};
use gpui::{
    Entity, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, img,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::{
    DockController,
    components::{context_menu::GroupMenuToggle, tab_overflow::TabOverflow},
    dock_components::GroupPresentation,
};

/// Reusable tab header. The shared Main/Floating Host pointer pipelines
/// handle activation and drag from Core tab geometry. Leaving the tab strip
/// transfers the same cached PanelView into the temporary DragHost; the logical
/// Panel stays in its original Group until the final drop command is dispatched.
pub struct DockTab;

impl DockTab {
    /// Computes the drag zone of the new DockColumn group header.
    ///
    /// It contains the tab, the flexible spacer, and the GroupGrip. Only the
    /// trailing GroupMenuButton is excluded from tab dragging.
    pub(crate) fn drag_zone(group_rect: Rect, metrics: DockMetrics) -> Rect {
        Rect::new(
            group_rect.origin,
            dock_core::Size::new(
                (group_rect.size.width - metrics.group_control_button_width).max(0.0),
                metrics
                    .header_height
                    .max(metrics.tab_strip_height)
                    .min(group_rect.size.height),
            ),
        )
    }

    /// Returns true once the pointer leaves the complete DockTab drag zone.
    pub(crate) fn leaves_strip_zone(strip_rect: Rect, point: Point) -> bool {
        !matches!(strip_rect.contains(point), Ok(true))
    }

    /// Renders the visual tab surface from Core geometry. Activation and drag are
    /// handled by the shared host pointer pipeline; closing lives in group controls.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_core(
        group: NodeId,
        header: TabHeaderRect,
        strip_rect: Rect,
        title: SharedString,
        icon: Option<SharedString>,
        active: bool,
        theme: Theme,
    ) -> impl IntoElement {
        let left = header.rect.origin.x - strip_rect.origin.x;
        let top = header.rect.origin.y - strip_rect.origin.y;
        let width = header.rect.size.width.max(0.0);
        let height = header.rect.size.height.max(0.0);

        let mut title_row = div()
            .absolute()
            .left(px(6.0))
            .right_0()
            .top_0()
            .h_full()
            .flex()
            .items_center()
            .overflow_hidden()
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size));

        if let Some(icon) = icon {
            title_row = title_row.child(img(icon).size(px(14.0)).flex_none());
        }

        title_row = title_row.child(
            div()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(title),
        );

        let tab = div()
            .id(format!("tab-{:?}-{:?}", group, header.panel))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .overflow_hidden()
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .bg(rgb(if active {
                theme.colors.panel_tab_active
            } else {
                theme.colors.panel_tab_inactive
            }))
            .text_color(rgb(theme.colors.primary_text))
            .cursor_pointer()
            .child(title_row);

        tab
    }

    pub fn render(
        id: String,
        title: String,
        active: bool,
        theme: Theme,
    ) -> impl IntoElement {
        Self::render_with_metrics(id, title, active, theme, 112.0, 72.0, 8.0)
    }

    pub fn render_with_width(
        id: String,
        title: String,
        active: bool,
        theme: Theme,
        width: f32,
    ) -> impl IntoElement {
        Self::render_with_metrics(id, title, active, theme, width, width.min(72.0), 8.0)
    }

    pub fn render_with_metrics(
        id: String,
        title: String,
        active: bool,
        theme: Theme,
        width: f32,
        min_width: f32,
        horizontal_padding: f32,
    ) -> impl IntoElement {
        div()
            .id(id)
            .w(px(width))
            .min_w(px(min_width))
            .h_full()
            .flex_none()
            .flex()
            .items_center()
            .cursor_move()
            .px(px(horizontal_padding))
            .overflow_hidden()
            .bg(rgb(if active {
                theme.colors.panel_tab_active
            } else {
                theme.colors.panel_tab_inactive
            }))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border))
            .text_color(rgb(theme.colors.primary_text))
            .text_size(px(theme.typography.tab_size))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            )
    }
}

const SHOW_GROUP_CONTROLS: bool = true;

/// Renders one Group inside a Column.
pub(crate) struct DockGroupView;

impl DockGroupView {
    /// Renders a static placeholder for a Group whose content is mounted by DragHost.
    pub(crate) fn render_placeholder(
        group: GroupRect,
        content_origin: Point,
        metrics: DockMetrics,
        theme: Theme,
    ) -> impl IntoElement {
        super::group_surface(group, content_origin, metrics, theme)
    }

    /// Renders a live Group with one active tab and its panel view.
    pub(super) fn render_active(
        id: String,
        title: String,
        panel_view: gpui::AnyView,
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

    /// Renders a Core Group with its complete tab strip and group controls.
    ///
    /// The geometry and tab presentation are supplied by dock-core and the host.
    /// This keeps the new DockColumn renderer on the same interactive path for
    /// one-tab, multi-tab and multi-group Columns.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_core(
        column: Option<NodeId>,
        group: GroupRect,
        strip: TabStripRect,
        headers: &[TabHeaderRect],
        overflow: Option<TabOverflowRect>,
        presentation: &GroupPresentation,
        content_origin: Point,
        metrics: DockMetrics,
        reserved_grip_left: f32,
        controller: Entity<DockController>,
        theme: Theme,
        toggle_menu: GroupMenuToggle,
    ) -> impl IntoElement {
        let left = group.rect.origin.x - content_origin.x;
        let top = group.rect.origin.y - content_origin.y;
        let header_height = if presentation.collapsed {
            0.0
        } else {
            metrics.header_height.min(group.rect.size.height)
        };
        let control_width = metrics
            .group_control_button_width
            .min(group.rect.size.width / 3.0)
            .max(1.0);

        let mut root = div()
            .id(format!("dock-group-{:?}", group.group))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(group.rect.size.width))
            .h(px(group.rect.size.height))
            .bg(rgb(theme.colors.group_frame_background))
            .border_1()
            .border_color(rgb(theme.colors.group_frame_border));

        if column.is_none() && header_height > 0.0 {
            let grip_width =
                (group.rect.size.width - reserved_grip_left - 2.0 * control_width).max(0.0);
            let grip = div()
                .id(format!("group-grip-{:?}", group.group))
                .absolute()
                .top_0()
                .left(px(reserved_grip_left))
                .w(px(grip_width))
                .h(px(header_height))
                .child(GroupGrip::render_with_metrics(
                    format!("group-grip-visual-{:?}", group.group),
                    theme,
                    grip_width,
                    metrics.control_icon_size,
                ));
            root = root.child(grip);
        }

        let mut strip_root = div()
            .id(format!("tab-strip-{:?}", group.group))
            .absolute()
            .left(px(strip.rect.origin.x - group.rect.origin.x))
            .top(px(strip.rect.origin.y - group.rect.origin.y))
            .w(px(strip.rect.size.width))
            .h(px(strip.rect.size.height))
            .flex()
            .items_stretch()
            .bg(rgb(theme.colors.tab_strip_background))
            .child(GroupDragArea::render(
                format!("group-drag-area-{:?}", group.group),
                theme,
            ));

        // Paint inactive tabs first so that the active tab remains in front.
        let mut ordered_headers = headers.to_vec();
        ordered_headers.sort_by_key(|header| presentation.active_panel == Some(header.panel));
        for header in ordered_headers {
            let tab = presentation
                .tabs
                .iter()
                .find(|tab| tab.panel == header.panel && tab.index == header.index);
            let title = tab
                .map(|tab| tab.title.clone())
                .unwrap_or_else(|| format!("Panel {}", header.index + 1).into());
            let icon = tab.and_then(|tab| tab.icon.clone());
            let active = presentation.active_panel == Some(header.panel);

            strip_root = strip_root.child(DockTab::render_core(
                group.group,
                header,
                strip.rect,
                title,
                icon,
                active,
                theme,
            ));
        }

        if let Some(overflow) = overflow {
            strip_root = strip_root.child(TabOverflow::render(strip.rect, overflow, theme));
        }

        root = root.child(strip_root);

        // Collapsed groups have no header rect, so their controls overlay the
        // trailing edge of the remaining tab strip to keep Expand/Menu reachable.
        let controls_top = if column.is_some() || presentation.collapsed {
            strip.rect.origin.y - group.rect.origin.y
        } else {
            0.0
        };
        let controls_height = if column.is_some() || presentation.collapsed {
            strip.rect.size.height
        } else {
            header_height
        };

        let menu_button = div()
            .absolute()
            .top(px(controls_top))
            .right_0()
            .w(px(control_width))
            .h(px(controls_height))
            .child(GroupMenuButton::render_for_group(
                format!("group-menu-button-{:?}", group.group),
                group.group,
                theme,
                control_width,
                metrics.control_icon_size,
                if column.is_some() { "☰" } else { "⋮" },
                toggle_menu.clone(),
            ));
        if column.is_some() {
            // Column Groups use one compact toolbar: tabs on the left, a
            // flexible drag lane in the middle, a fixed Group grip, and Menu.
            let group_grip = div()
                .absolute()
                .left(px((group.rect.size.width - 2.0 * control_width).max(0.0)))
                .top(px(controls_top))
                .w(px(control_width))
                .h(px(controls_height))
                .child(GroupGrip::render_with_metrics(
                    format!("group-grip-handle-{:?}", group.group),
                    theme,
                    control_width,
                    metrics.control_icon_size,
                ));
            return root.child(group_grip).child(menu_button);
        }

        root = root.child(menu_button);

        let collapse_controller = controller.clone();
        let collapse_group = group.group;
        let collapse_to = !presentation.collapsed;
        let model_collapsed = presentation.model_collapsed;
        let auto_collapsed = presentation.auto_collapsed;
        let auto_expanded = presentation.auto_expanded;
        let collapse_button = div()
            .id(format!("group-collapse-button-{:?}", group.group))
            .absolute()
            .top(px(controls_top))
            .right(px(control_width))
            .w(px(control_width))
            .h(px(controls_height))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.group_menu_button))
            .text_color(rgb(theme.colors.primary_text))
            .child(if presentation.collapsed { "＋" } else { "−" })
            .on_click(move |_, _, app| {
                let mut commands = Vec::with_capacity(2);
                if auto_collapsed {
                    if model_collapsed {
                        commands.push(DockCommand::CollapseGroup {
                            group: collapse_group,
                            collapsed: false,
                        });
                    }
                    if let Some(column) = column {
                        commands.push(DockCommand::SetColumnExpandedGroup {
                            column,
                            group: Some(collapse_group),
                        });
                    } else if commands.is_empty() {
                        commands.push(DockCommand::CollapseGroup {
                            group: collapse_group,
                            collapsed: false,
                        });
                    }
                } else if auto_expanded {
                    commands.push(DockCommand::CollapseGroup {
                        group: collapse_group,
                        collapsed: true,
                    });
                    if let Some(column) = column {
                        commands.push(DockCommand::SetColumnExpandedGroup {
                            column,
                            group: None,
                        });
                    }
                } else {
                    commands.push(DockCommand::CollapseGroup {
                        group: collapse_group,
                        collapsed: collapse_to,
                    });
                }

                collapse_controller.update(app, |controller, controller_cx| {
                    if let Err(error) =
                        controller.dispatch_sequence_in(commands, controller_cx)
                    {
                        tracing::warn!(%error, "could not change dock Group expansion");
                    }
                });
            });
        root = root.child(collapse_button);

        root
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

#[cfg(test)]
mod tests {
    use dock_core::{DockMetrics, Point, Rect, Size};

    use super::DockTab;

    #[test]
    fn tab_drag_zone_includes_the_group_grip_but_excludes_the_group_menu() {
        let group = Rect::new(Point::new(10.0, 20.0), Size::new(240.0, 160.0));
        let zone = DockTab::drag_zone(group, DockMetrics::default());

        assert_eq!(zone.origin, Point::new(10.0, 20.0));
        assert_eq!(zone.size, Size::new(212.0, 28.0));
        assert!(!DockTab::leaves_strip_zone(zone, Point::new(20.0, 30.0)));
        // The blank lane remains part of the drag zone.
        assert!(!DockTab::leaves_strip_zone(zone, Point::new(180.0, 30.0)));
        // The GroupGrip (32 px) is included; the 28 px GroupMenuButton is not.
        assert!(!DockTab::leaves_strip_zone(zone, Point::new(215.0, 30.0)));
        assert!(DockTab::leaves_strip_zone(zone, Point::new(222.0, 30.0)));
        assert!(DockTab::leaves_strip_zone(zone, Point::new(20.0, 48.0)));
    }
}
