use dock_core::Point;

#[derive(Default)]
pub(super) struct ResizeFrameThrottle {
    pub(super) latest: Option<Point>,
    pub(super) scheduled: bool,
}
