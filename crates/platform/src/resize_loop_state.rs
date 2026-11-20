use crate::{NativeResizeProposal, PhysicalPoint, PhysicalRect, PhysicalSize};
use windows::Win32::Foundation::{HWND, POINT};

pub(super) struct ResizeLoopState {
    pub(super) active: bool,
    pub(super) horizontal_edge: Option<crate::NativeHorizontalResizeEdge>,
    pub(super) left_resize_enabled: bool,
    pub(super) right_resize_enabled: bool,
    pub(super) top_resize_enabled: bool,
    pub(super) bottom_resize_enabled: bool,
    pub(super) chain_z_order: Vec<HWND>,
    pub(super) resize_followers: Vec<HWND>,
    pub(super) resize_follower_origins: Vec<(HWND, POINT)>,
    pub(super) resize_start_right: Option<i32>,
    pub(super) left_min_client_width: i32,
    pub(super) right_min_client_width: i32,
    pub(super) resize_sequence: u64,
    pub(super) resize_revision: u64,
    pub(super) initial_cursor: Option<PhysicalPoint>,
    pub(super) initial_window_rect: Option<PhysicalRect>,
    pub(super) initial_client_origin: Option<PhysicalPoint>,
    pub(super) initial_client_size: Option<PhysicalSize>,
    pub(super) initial_scale_factor: f32,
    pub(super) latest_resize_proposal: Option<NativeResizeProposal>,
}

impl ResizeLoopState {
    pub(super) fn new() -> Self {
        Self {
            active: false,
            horizontal_edge: None,
            left_resize_enabled: true,
            right_resize_enabled: true,
            top_resize_enabled: true,
            bottom_resize_enabled: true,
            chain_z_order: Vec::new(),
            resize_followers: Vec::new(),
            resize_follower_origins: Vec::new(),
            resize_start_right: None,
            left_min_client_width: 0,
            right_min_client_width: 0,
            resize_sequence: 0,
            resize_revision: 0,
            initial_cursor: None,
            initial_window_rect: None,
            initial_client_origin: None,
            initial_client_size: None,
            initial_scale_factor: 1.0,
            latest_resize_proposal: None,
        }
    }
}
