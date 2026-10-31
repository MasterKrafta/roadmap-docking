use dock_core::{
    DockCommand, DragHandle, DragPayload, FloatingGroupId, GroupRect, Point, TabHeaderRect,
    TabOverflowRect, TabStripRect,
};
use gpui::{
    Context, Entity, IntoElement, MouseButton, ParentElement, Render, Styled, Window, div, px,
    rgb,
    prelude::InteractiveElement as _,
};
use ui::Theme;

use crate::{
    DockController,
    input::{begin_drag, dispatch_command, finish_drag, update_drag_at},
};

use super::{
    context_menu::{ContextMenuTarget, GroupMenuToggle},
    tab_strip::{GroupPresentation, TabStripView},
};

/// GPUI drag marker for the complete Tabs group grip.
#[derive(Clone, Copy)]
struct GroupGripDrag;

struct GroupGripDragGhost;

impl Render for GroupGripDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

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
            let grip_controller = controller.clone();
            let grip_group = group.group;
            let grip = div()
                .id(format!("group-grip-{:?}", group.group))
                .absolute()
                .top_0()
                .left(px(reserved_grip_left))
                .w(px(
                    (group.rect.size.width - reserved_grip_left - 2.0 * control_width).max(0.0),
                ))
                .h(px(header_height))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(rgb(theme.colors.group_grip_background))
                .text_color(rgb(theme.colors.primary_text))
                .child("⠿")
                .on_mouse_down(MouseButton::Left, move |event, _, app| {
                    begin_drag(
                        &grip_controller,
                        DragPayload::Group { group: grip_group },
                        DragHandle::GroupGrip,
                        event,
                        app,
                    );
                })
                .on_drag(GroupGripDrag, |_, _, _, app| {
                    app.new(|_| GroupGripDragGhost)
                })
                .on_drag_move::<GroupGripDrag>({
                    let controller = controller.clone();
                    move |event, _, app| {
                        update_drag_at(
                            &controller,
                            Point::new(
                                event.event.position.x.as_f32(),
                                event.event.position.y.as_f32(),
                            ),
                            app,
                        );
                    }
                })
                .on_mouse_up({
                    let controller = controller.clone();
                    move |event, _, app| finish_drag(&controller, event, app)
                })
                .on_mouse_up_out(MouseButton::Left, {
                    let controller = controller.clone();
                    move |event, _, app| finish_drag(&controller, event, app)
                });
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
            toggle_menu.clone(),
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
                    Point::new(
                        event.position().x.as_f32(),
                        event.position().y.as_f32(),
                    ),
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
