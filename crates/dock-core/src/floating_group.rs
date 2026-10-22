use crate::{DockAreaId, GeometryError, Rect};

/// Metadata for one detached floating area.
///
/// The associated tree root is stored once in the DockArea arena; this type
/// adds only window-independent bounds and stacking order.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingGroup {
    pub area: DockAreaId,
    pub bounds: Rect,
    pub z_order: i32,
}

impl FloatingGroup {
    pub const fn new(area: DockAreaId, bounds: Rect, z_order: i32) -> Self {
        Self {
            area,
            bounds,
            z_order,
        }
    }

    /// Floating windows must have finite, positive bounds.
    pub fn validate(&self) -> Result<(), GeometryError> {
        self.bounds.validate_non_empty()
    }
}

#[cfg(test)]
mod tests {
    use crate::{DockAreaId, FloatingGroup, GeometryError, Point, Rect, Size};
    use slotmap::SlotMap;

    #[test]
    fn accepts_finite_positive_bounds() {
        let mut areas = SlotMap::<DockAreaId, ()>::with_key();
        let area = areas.insert(());
        let group = FloatingGroup::new(
            area,
            Rect::new(Point::new(12.0, 18.0), Size::new(640.0, 480.0)),
            3,
        );
        assert_eq!(group.validate(), Ok(()));
    }

    #[test]
    fn rejects_empty_or_non_finite_bounds() {
        let mut areas = SlotMap::<DockAreaId, ()>::with_key();
        let area = areas.insert(());
        let empty = FloatingGroup::new(area, Rect::new(Point::ZERO, Size::ZERO), 0);
        assert_eq!(empty.validate(), Err(GeometryError::NonPositiveDimension));

        let invalid = FloatingGroup::new(
            area,
            Rect::new(Point::ZERO, Size::new(f32::INFINITY, 100.0)),
            0,
        );
        assert_eq!(invalid.validate(), Err(GeometryError::NonFinite));
    }
}
