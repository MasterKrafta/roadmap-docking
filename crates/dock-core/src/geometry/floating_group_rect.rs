use crate::geometry::Rect;
use crate::ids::FloatingGroupId;

/// Bounds of an in-window floating group after clipping to the client area.
///
/// The collection in LayoutRects is sorted by ascending z_order, so later entries
/// are painted above earlier ones.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingGroupRect {
    pub floating: FloatingGroupId,
    pub bounds: Rect,
    pub z_order: u64,
}

#[cfg(test)]
mod tests {
    use super::FloatingGroupRect;
    use crate::{FloatingGroupId, Point, Rect, Size};

    #[test]
    fn floating_group_rect_retains_bounds_and_stack_order() {
        let rect = FloatingGroupRect {
            floating: FloatingGroupId::null(),
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(320.0, 240.0)),
            z_order: 3,
        };
        assert_eq!(rect.bounds.size, Size::new(320.0, 240.0));
        assert_eq!(rect.z_order, 3);
    }
}
