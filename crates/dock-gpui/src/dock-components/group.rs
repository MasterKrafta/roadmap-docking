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

use dock_core::{
    DockCommand, DockMetrics, GroupRect, NodeId, Point, TabHeaderRect,
    TabOverflowRect, TabStripRect,
};
use gpui::{
    Entity, IntoElement, ParentElement, StatefulInteractiveElement, Styled, div,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::{
    DockController,
    components::{
        context_menu::GroupMenuToggle,
        tab_overflow::TabOverflow,
        tab_strip::GroupPresentation,
    },
};

const SHOW_GROUP_CONTROLS: bool = true;

/// Renders one Group inside a Column.
pub(crate) struct DockGroupView;

impl DockGroupView {
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
