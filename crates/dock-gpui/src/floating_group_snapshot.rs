use dock_core::{FloatingGroupId, PanelId, Rect, Size};

#[derive(Clone)]
pub(super) struct FloatingGroupSnapshot {
    pub(super) id: FloatingGroupId,
    pub(super) bounds: Rect,
    pub(super) z_order: i32,
    pub(super) minimum_size: Option<Size>,
    pub(super) horizontal_resize_minimum_widths: Option<(f32, f32)>,
    pub(super) panels: Vec<PanelId>,
}
