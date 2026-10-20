use crate::command::DropTarget;
use crate::geometry::Rect;

/// A candidate drop target and the logical rectangle that activates it.
///
/// The target includes the semantic destination and its DropZone where applicable.
/// Hit-testing and zone resolution are kept outside this data type.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DropZoneRect {
    pub target: DropTarget,
    pub bounds: Rect,
}

#[cfg(test)]
mod tests {
    use super::DropZoneRect;
    use crate::{DockSide, DropTarget, DropZone, NodeId, Point, Rect, Size};

    #[test]
    fn drop_zone_rect_keeps_target_and_logical_bounds_together() {
        let bounds = Rect::new(Point::new(100.5, 40.25), Size::new(120.0, 72.0));
        let zone = DropZoneRect {
            target: DropTarget::Group {
                group: NodeId::null(),
                zone: DropZone::Top,
            },
            bounds,
        };

        assert_eq!(
            zone.target,
            DropTarget::Group {
                group: NodeId::null(),
                zone: DropZone::Top,
            }
        );
        assert_eq!(zone.bounds, bounds);

        let edge = DropZoneRect {
            target: DropTarget::DockEdge { side: DockSide::Left },
            bounds,
        };
        assert_eq!(edge.bounds, bounds);
    }
}
