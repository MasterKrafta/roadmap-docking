//! GPUI event adapters. UI events are translated into dock-core input values here.

pub mod drag;
pub mod pointer;
pub mod resize;
pub mod scroll;

pub(crate) use drag::floating_bounds;
pub(crate) use pointer::{PointerEventState, PointerPress, register_pointer_events};
pub(crate) use resize::{ResizePreview, ResizePreviewKind, resize_preview_for_interaction};
pub(crate) use scroll::{map_hit_test_point, scroll_adjusted_hit_test_point};
