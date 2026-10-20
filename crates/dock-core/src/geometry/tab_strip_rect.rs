use crate::geometry::Rect;
use crate::ids::NodeId;

/// Logical bounds of the complete tab strip for a dock group.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TabStripRect {
    pub group: NodeId,
    pub bounds: Rect,
}

#[cfg(test)]
mod tests {
    use super::TabStripRect;
    use crate::{NodeId, Point, Rect, Size};
    use slotmap::Key;

    #[test]
    fn tab_strip_rect_keeps_group_and_strip_height() {
        let strip = TabStripRect {
            group: NodeId::null(),
            bounds: Rect::new(Point::new(1000.0, 32.0), Size::new(280.0, 28.0)),
        };
        assert_eq!(strip.group, NodeId::null());
        assert_eq!(strip.bounds.size.height, 28.0);
    }
}
