use crate::geometry::Rect;
use crate::ids::NodeId;

/// Logical bounds of one dock group, from its tab strip through panel content.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupRect {
    pub group: NodeId,
    pub bounds: Rect,
}

#[cfg(test)]
mod tests {
    use super::GroupRect;
    use crate::{NodeId, Point, Rect, Size};

    #[test]
    fn group_rect_keeps_its_node_identity() {
        let group = GroupRect {
            group: NodeId::null(),
            bounds: Rect::new(Point::new(0.0, 32.0), Size::new(280.0, 382.0)),
        };
        assert_eq!(group.group, NodeId::null());
        assert_eq!(group.bounds.size.height, 382.0);
    }
}
