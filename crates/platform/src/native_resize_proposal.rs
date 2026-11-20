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
    /// Win32 WMSZ_* edge code (1 through 8) for this resize sample.
    pub sizing_edge: u32,
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
    /// Cursor-derived change in the client dimensions, in physical pixels.
    /// This is independent of the native HWND's measured or proposed bounds.
    pub const fn client_delta_from_cursor(self) -> PhysicalSize {
        let dx = self.cursor.x.saturating_sub(self.initial_cursor.x);
        let dy = self.cursor.y.saturating_sub(self.initial_cursor.y);
        PhysicalSize {
            width: match self.sizing_edge {
                1 | 4 | 7 => 0_i32.saturating_sub(dx),
                2 | 5 | 8 => dx,
                _ => 0,
            },
            height: match self.sizing_edge {
                3 | 4 | 5 => 0_i32.saturating_sub(dy),
                6 | 7 | 8 => dy,
                _ => 0,
            },
        }
    }

    /// Cursor-derived movement of the outer window's top-left corner.
    pub const fn origin_delta_from_cursor(self) -> PhysicalPoint {
        let dx = self.cursor.x.saturating_sub(self.initial_cursor.x);
        let dy = self.cursor.y.saturating_sub(self.initial_cursor.y);
        PhysicalPoint {
            x: match self.sizing_edge {
                1 | 4 | 7 => dx,
                _ => 0,
            },
            y: match self.sizing_edge {
                3 | 4 | 5 => dy,
                _ => 0,
            },
        }
    }
}
