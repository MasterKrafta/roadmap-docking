use dock_core::{
    DockCommand, FloatingGroupId, GroupRect, Point, TabHeaderRect, TabOverflowRect, TabStripRect,
};
use gpui::{
    Entity, IntoElement, ParentElement, StatefulInteractiveElement, Styled, div,
    prelude::InteractiveElement as _, px, rgb,
};
use ui::Theme;

use crate::{DockController, actions::dispatch_command};

use super::{
    context_menu::{ContextMenuTarget, GroupMenuToggle},
    tab_strip::{GroupPresentation, TabStripView},
};

/// The group component: grip, tab strip, controls, and group context menu.
pub struct GroupView;

impl GroupView {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        group: GroupRect,
        strip: TabStripRect,
        headers: &[TabHeaderRect],
        overflow: Option<TabOverflowRect>,
        presentation: &GroupPresentation,
        content_origin: Point,
        metrics: dock_core::DockMetrics,
        reserved_grip_left: f32,
        controller: Entity<DockController>,
        theme: Theme,
        raise_floating: Option<FloatingGroupId>,
        toggle_menu: GroupMenuToggle,
        middle_click_closes_tabs: bool,
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

        if header_height > 0.0 {
            let grip = div()
                .id(format!("group-grip-{:?}", group.group))
                .absolute()
                .top_0()
                .left(px(reserved_grip_left))
                .w(px((group.rect.size.width
                    - reserved_grip_left
                    - 2.0 * control_width)
                    .max(0.0)))
                .h(px(header_height))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(rgb(theme.colors.group_grip_background))
                .text_color(rgb(theme.colors.primary_text))
                .child("⠿");
            root = root.child(grip);
        }

        root = root.child(TabStripView::render(
            group.group,
            group.rect,
            strip,
            headers,
            overflow,
            presentation,
            controller.clone(),
            theme,
            raise_floating,
            middle_click_closes_tabs,
        ));

        // Collapsed groups have no header rect, so their controls overlay the
        // trailing edge of the remaining tab strip to keep Expand/Menu reachable.
        let controls_top = if presentation.collapsed {
            strip.rect.origin.y - group.rect.origin.y
        } else {
            0.0
        };
        let controls_height = if presentation.collapsed {
            strip.rect.size.height
        } else {
            header_height
        };

        let menu_toggle = toggle_menu.clone();
        let menu_button = div()
            .id(format!("group-menu-button-{:?}", group.group))
            .absolute()
            .top(px(controls_top))
            .right(px(0.0))
            .w(px(control_width))
            .h(px(controls_height))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .bg(rgb(theme.colors.group_menu_button))
            .text_color(rgb(theme.colors.primary_text))
            .child("⋮")
            .on_click(move |event, _, app| {
                (menu_toggle)(
                    ContextMenuTarget::Group(group.group),
                    match crate::geometry::point_from_gpui(event.position()) {
                        Ok(anchor) => anchor,
                        Err(_) => return,
                    },
                    app,
                );
            });
        root = root.child(menu_button);

        let collapse_controller = controller.clone();
        let collapse_group = group.group;
        let collapse_to = !presentation.collapsed;
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
                dispatch_command(
                    &collapse_controller,
                    DockCommand::CollapseGroup {
                        group: collapse_group,
                        collapsed: collapse_to,
                    },
                    app,
                );
            });
        root = root.child(collapse_button);

        root
    }
}
