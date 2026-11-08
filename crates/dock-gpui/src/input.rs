//! GPUI event adapters. UI events are translated into dock-core input values here.

#[path = "input_drag.rs"]
pub mod drag;
#[path = "input_pointer.rs"]
pub mod pointer;
#[path = "input_resize.rs"]
pub mod resize;
#[path = "input_scroll.rs"]
pub mod scroll;
#[path = "input_interaction.rs"]
pub(crate) mod interaction;

pub(crate) use drag::floating_bounds;
pub(crate) use pointer::{PointerEventState, PointerPress, register_pointer_events};
pub(crate) use resize::{ResizePreview, ResizePreviewKind, resize_preview_for_interaction};
pub(crate) use scroll::{map_hit_test_point, scroll_adjusted_hit_test_point};
