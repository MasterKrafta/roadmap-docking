use crate::geometry::Rect;
use crate::model::DockSide;

/// Logical bounds of a dock column, including its shared column header.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ColumnRect {
    pub side: DockSide,
    pub index: usize,
    /// True when the group stack is taller than the viewport and needs vertical scrolling.
    pub vertical_overflow: bool,
    pub bounds: Rect,
}

#[cfg(test)]
mod tests {
    use super::ColumnRect;
    use crate::{DockSide, Point, Rect, Size};

    #[test]
    fn column_rect_retains_side_index_bounds_and_overflow_flag() {
        let column = ColumnRect {
            side: DockSide::Right,
            index: 1,
            bounds: Rect::new(Point::new(1000.0, 0.0), Size::new(280.0, 800.0)),
            vertical_overflow: true,
        };
        assert_eq!(column.side, DockSide::Right);
        assert_eq!(column.index, 1);
        assert_eq!(column.bounds.origin.x, 1000.0);
        assert!(column.vertical_overflow);
    }
}
