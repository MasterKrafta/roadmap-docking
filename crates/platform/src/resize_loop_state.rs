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
}
