//! GPUI event adapters. UI events are translated into dock-core input values here.

pub mod commands;
pub mod drag;
pub mod scroll;
pub mod resize;
pub mod pointer;

pub(crate) use commands::dispatch_command;
pub(crate) use drag::{begin_drag_at, finish_drag_at, floating_bounds, update_drag_at};
pub(crate) use scroll::{map_hit_test_point, scroll_adjusted_hit_test_point};
pub(crate) use resize::{
    begin_resize_hit, finish_resize, resize_preview_for_interaction, update_resize,
    ResizePreview, ResizePreviewKind,
};
pub(crate) use pointer::{PointerPress, register_pointer_events};
