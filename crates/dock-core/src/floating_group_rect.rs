use crate::{FloatingGroupId, Rect};

/// Logical bounds and paint/hit-test stacking key for a floating dock group.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingGroupRect {
    pub group: FloatingGroupId,
    pub rect: Rect,
    pub z_order: i32,
}

impl FloatingGroupRect {
    pub const fn new(group: FloatingGroupId, rect: Rect, z_order: i32) -> Self {
        Self {
            group,
            rect,
            z_order,
        }
    }
}

#[cfg(test)]
mod tests {
    use slotmap::Key;

    use super::FloatingGroupRect;
    use crate::{FloatingGroupId, Point, Rect, Size};

    #[test]
    fn floating_geometry_keeps_z_order_with_bounds() {
        let group = FloatingGroupRect::new(
            FloatingGroupId::null(),
            Rect::new(Point::new(10.0, 20.0), Size::new(300.0, 200.0)),
            7,
        );
        assert_eq!(group.rect.origin, Point::new(10.0, 20.0));
        assert_eq!(group.z_order, 7);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn floating_geometry_round_trips_through_serde() {
        let group = FloatingGroupRect::new(
            FloatingGroupId::null(),
            Rect::new(Point::new(10.0, 20.0), Size::new(300.0, 200.0)),
            7,
        );
        let encoded = serde_json::to_string(&group).unwrap();
        assert_eq!(
            serde_json::from_str::<FloatingGroupRect>(&encoded).unwrap(),
            group
        );
    }
}
