use crate::{GeometryError, Rect};

/// Persistent geometry for the protected canvas outside the dock tree.
///
/// Bounds are optional until the first layout measurement. Canvas content such
/// as document selection, zoom, or application-specific scene state does not
/// belong in dock-core.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CanvasState {
    pub last_known_bounds: Option<Rect>,
}

impl CanvasState {
    pub const fn new(last_known_bounds: Option<Rect>) -> Self {
        Self { last_known_bounds }
    }

    pub const fn last_known_bounds(&self) -> Option<Rect> {
        self.last_known_bounds
    }

    pub fn validate(&self) -> Result<(), GeometryError> {
        match self.last_known_bounds {
            Some(bounds) => bounds.validate_non_empty(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{CanvasState, GeometryError, Point, Rect, Size};

    #[test]
    fn canvas_can_exist_before_its_bounds_are_measured() {
        assert_eq!(CanvasState::default().validate(), Ok(()));
        assert_eq!(CanvasState::default().last_known_bounds(), None);
    }

    #[test]
    fn canvas_bounds_must_be_positive_and_finite() {
        let valid = CanvasState::new(Some(Rect::new(
            Point::new(200.0, 0.0),
            Size::new(800.0, 600.0),
        )));
        assert_eq!(valid.validate(), Ok(()));

        let empty = CanvasState::new(Some(Rect::new(Point::ZERO, Size::ZERO)));
        assert_eq!(empty.validate(), Err(GeometryError::NonPositiveDimension));
    }
}
