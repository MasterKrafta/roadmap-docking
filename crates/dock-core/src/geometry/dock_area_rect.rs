use crate::geometry::Rect;
use crate::model::DockSide;

/// Logical bounds of one dock area on the left or right of the Canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DockAreaRect {
    pub side: DockSide,
    pub bounds: Rect,
}

#[cfg(test)]
mod tests {
    use super::DockAreaRect;
    use crate::{DockSide, Point, Rect, Size};

    #[test]
    fn dock_area_rect_retains_side_and_bounds() {
        let area = DockAreaRect {
            side: DockSide::Left,
            bounds: Rect::new(Point::new(0.0, 0.0), Size::new(280.0, 800.0)),
        };
        assert_eq!(area.side, DockSide::Left);
        assert_eq!(area.bounds.size, Size::new(280.0, 800.0));
    }
}
