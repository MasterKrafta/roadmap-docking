//! GPUI event adapters. UI events are translated into dock-core input values here.

pub mod commands;
pub mod drag;
pub mod scroll;
pub mod resize;

pub(crate) use commands::dispatch_command;
pub(crate) use drag::{
    begin_drag, begin_drag_at, finish_drag, finish_drag_at, floating_bounds, update_drag_at,
};
pub(crate) use scroll::scroll_adjusted_hit_test_point;
pub(crate) use resize::{begin_resize, finish_resize, update_resize};
