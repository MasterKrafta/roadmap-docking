//! GPUI event adapters. UI events are translated into dock-core input values here.

pub mod drag;
pub mod scroll;
pub mod resize;
pub mod pointer;

pub(crate) use drag::floating_bounds;
pub(crate) use scroll::{map_hit_test_point, scroll_adjusted_hit_test_point};
pub(crate) use resize::{resize_preview_for_interaction, ResizePreview, ResizePreviewKind};
pub(crate) use pointer::{PointerPress, register_pointer_events};
