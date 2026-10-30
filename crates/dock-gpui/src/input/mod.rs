//! GPUI event adapters. UI events are translated into dock-core input values here.

pub mod commands;
pub mod drag;
pub mod scroll;

pub(crate) use commands::dispatch_command;
pub(crate) use drag::{begin_drag, finish_drag, update_drag};
pub(crate) use scroll::scroll_adjusted_hit_test_point;
