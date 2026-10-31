use std::{
    cell::RefCell,
    rc::Rc,
};

use dock_core::{Axis, Point, SplitterRect};
use gpui::{
    Context, Entity, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement,
    Render, Styled, Window, div, px, rgb,
    prelude::{InteractiveElement as _, StatefulInteractiveElement as _},
};
use ui::Theme;

use crate::{
    DockController,
    input::{begin_resize, finish_resize, update_resize},
};

#[derive(Clone, Copy)]
struct SplitterDrag;

struct SplitterDragGhost;

impl Render for SplitterDragGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Default)]
struct FrameThrottle {
    latest: Option<Point>,
    scheduled: bool,
}

/// Interactive splitter hit target and its visible, centered handle line.
pub struct SplitterView;

impl SplitterView {
    pub(crate) fn render(
        splitter: SplitterRect,
        content_origin: Point,
        pointer_offset: Point,
        controller: Entity<DockController>,
        theme: Theme,
    ) -> impl IntoElement {
        let hit_rect = splitter.rect;
        let thickness = theme.metrics.splitter_thickness;
        let (left, top, width, height, line_left, line_top, line_width, line_height) =
            match splitter.axis {
                Axis::X => (
                    hit_rect.origin.x - content_origin.x,
                    hit_rect.origin.y - content_origin.y,
                    hit_rect.size.width,
                    hit_rect.size.height,
                    (hit_rect.size.width - thickness) * 0.5,
                    0.0,
                    thickness,
                    hit_rect.size.height,
                ),
                Axis::Y => (
                    hit_rect.origin.x - content_origin.x,
                    hit_rect.origin.y - content_origin.y,
                    hit_rect.size.width,
                    hit_rect.size.height,
                    0.0,
                    (hit_rect.size.height - thickness) * 0.5,
                    hit_rect.size.width,
                    thickness,
                ),
            };

        let throttle = Rc::new(RefCell::new(FrameThrottle::default()));
        let controller_for_down = controller.clone();
        let controller_for_drag = controller.clone();
        let controller_for_up = controller.clone();
        let controller_for_up_out = controller.clone();
        let throttle_for_up = throttle.clone();
        let throttle_for_up_out = throttle.clone();

        let mut hitbox = div()
            .id(format!(
                "splitter-{:?}-{}-{:?}",
                splitter.owner, splitter.boundary_index, splitter.axis
            ))
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, app| {
                begin_resize(
                    &controller_for_down,
                    splitter,
                    event_point(event.position, pointer_offset),
                    app,
                );
                app.stop_propagation();
            })
            .on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, _, app| {
                throttle_for_up.borrow_mut().latest = None;
                finish_resize(
                    &controller_for_up,
                    event_point(event.position, pointer_offset),
                    event.click_count as usize,
                    app,
                );
                app.stop_propagation();
            })
            .on_mouse_up_out(MouseButton::Left, move |event: &MouseUpEvent, _, app| {
                throttle_for_up_out.borrow_mut().latest = None;
                finish_resize(
                    &controller_for_up_out,
                    event_point(event.position, pointer_offset),
                    event.click_count as usize,
                    app,
                );
                app.stop_propagation();
            })
            .on_drag(SplitterDrag, |_, _, _, app| app.new(|_| SplitterDragGhost))
            .on_drag_move::<SplitterDrag>({
                let throttle = throttle.clone();
                let controller = controller_for_drag.clone();
                move |event, window, _app| {
                    let point = event_point(event.event.position, pointer_offset);
                    let should_schedule = {
                        let mut state = throttle.borrow_mut();
                        state.latest = Some(point);
                        if state.scheduled {
                            false
                        } else {
                            state.scheduled = true;
                            true
                        }
                    };

                    if should_schedule {
                        let throttle = throttle.clone();
                        let controller = controller.clone();
                        window.on_next_frame(move |_, app| {
                            let point = {
                                let mut state = throttle.borrow_mut();
                                state.scheduled = false;
                                state.latest.take()
                            };
                            if let Some(point) = point {
                                update_resize(&controller, point, app);
                            }
                        });
                    }
                }
            });

        // The hitbox stays transparent; only its centered line is painted.
        hitbox = match splitter.axis {
            Axis::X => hitbox.cursor_col_resize(),
            Axis::Y => hitbox.cursor_row_resize(),
        };
        hitbox.child(
            div()
                .absolute()
                .left(px(line_left))
                .top(px(line_top))
                .w(px(line_width.max(0.0)))
                .h(px(line_height.max(0.0)))
                .bg(rgb(theme.colors.splitter_handle)),
        )
    }
}

fn event_point(point: gpui::Point<gpui::Pixels>, offset: Point) -> Point {
    Point::new(point.x.as_f32(), point.y.as_f32() + offset.y)
}
