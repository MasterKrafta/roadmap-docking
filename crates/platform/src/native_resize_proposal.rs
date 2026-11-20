use crate::{PhysicalPoint, PhysicalRect, PhysicalSize};

/// One accepted native sizing proposal shared by the window and its content layout.
///
/// The rectangle is the Win32 WM_SIZING result after native handlers and size
/// constraints have run. Cursor coordinates are captured from the same message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeResizeProposal {
    pub session_id: u64,
    pub revision: u64,
    pub active: bool,
    pub initial_cursor: PhysicalPoint,
    pub cursor: PhysicalPoint,
    pub initial_window_rect: PhysicalRect,
    pub window_rect: PhysicalRect,
    pub initial_client_origin: PhysicalPoint,
    pub client_origin: PhysicalPoint,
    pub initial_client_size: PhysicalSize,
    pub client_size: PhysicalSize,
    pub initial_scale_factor: f32,
    pub scale_factor: f32,
}

impl NativeResizeProposal {
    pub const fn client_width_delta(self) -> i32 {
        self.client_size.width - self.initial_client_size.width
    }

    pub const fn client_height_delta(self) -> i32 {
        self.client_size.height - self.initial_client_size.height
    }
}
