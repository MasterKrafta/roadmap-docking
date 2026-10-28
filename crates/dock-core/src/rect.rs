use crate::{GeometryError, Point, Size};

/// An axis-aligned rectangle in logical pixels.
///
/// The top and left edges are inclusive; the right and bottom edges are
/// exclusive: [x, x + width) × [y, y + height).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const ZERO: Self = Self {
        origin: Point::ZERO,
        size: Size::ZERO,
    };

    pub const fn new(origin: Point, size: Size) -> Self {
        Self { origin, size }
    }

    /// Validates finite coordinates, non-negative dimensions, and representable positive edges.
    pub fn validate(self) -> Result<(), GeometryError> {
        self.origin.validate()?;
        self.size.validate()?;
        self.checked_right()?;
        self.checked_bottom()?;
        Ok(())
    }

    /// Validates a rectangle that must have positive width and height.
    pub fn validate_non_empty(self) -> Result<(), GeometryError> {
        self.validate()?;
        self.size.validate_positive()
    }

    /// Computes the right edge, rejecting a positive width that rounds back to the origin.
    pub fn checked_right(self) -> Result<f32, GeometryError> {
        if !self.origin.x.is_finite() || !self.size.width.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if self.size.width < 0.0 {
            return Err(GeometryError::NegativeDimension);
        }
        let right = self.origin.x + self.size.width;
        if !right.is_finite() {
            return Err(GeometryError::ArithmeticOverflow);
        }
        // A finite positive width can disappear when added to a large f32
        // origin. Such a rectangle has no representable right edge.
        if self.size.width > 0.0 && right <= self.origin.x {
            return Err(GeometryError::NonPositiveDimension);
        }
        Ok(right)
    }

    /// Computes the bottom edge, rejecting a positive height that rounds back to the origin.
    pub fn checked_bottom(self) -> Result<f32, GeometryError> {
        if !self.origin.y.is_finite() || !self.size.height.is_finite() {
            return Err(GeometryError::NonFinite);
        }
        if self.size.height < 0.0 {
            return Err(GeometryError::NegativeDimension);
        }
        let bottom = self.origin.y + self.size.height;
        if !bottom.is_finite() {
            return Err(GeometryError::ArithmeticOverflow);
        }
        // Keep the vertical edge contract identical to the horizontal one.
        if self.size.height > 0.0 && bottom <= self.origin.y {
            return Err(GeometryError::NonPositiveDimension);
        }
        Ok(bottom)
    }

    /// Returns whether the point lies inside the half-open rectangle.
    pub fn contains(self, point: Point) -> Result<bool, GeometryError> {
        // Validate non-edge metadata once, then compute each checked edge once.
        self.origin.validate()?;
        self.size.validate()?;
        point.validate()?;
        let right = self.checked_right()?;
        let bottom = self.checked_bottom()?;
        Ok(point.x >= self.origin.x
            && point.x < right
            && point.y >= self.origin.y
            && point.y < bottom)
    }

    /// Returns whether two half-open rectangles overlap with positive area.
    /// Merely touching at an edge or corner is not an intersection.
    pub fn intersects(self, other: Self) -> Result<bool, GeometryError> {
        self.validate()?;
        other.validate()?;
        let self_right = self.checked_right()?;
        let self_bottom = self.checked_bottom()?;
        let other_right = other.checked_right()?;
        let other_bottom = other.checked_bottom()?;
        Ok(self.origin.x < other_right
            && other.origin.x < self_right
            && self.origin.y < other_bottom
            && other.origin.y < self_bottom)
    }
}

#[cfg(test)]
mod tests {
    use super::{GeometryError, Point, Rect, Size};

    #[test]
    fn contains_uses_half_open_edges() {
        let rect = Rect::new(Point::new(10.0, 20.0), Size::new(5.0, 4.0));
        assert_eq!(rect.contains(Point::new(10.0, 20.0)), Ok(true));
        assert_eq!(rect.contains(Point::new(14.999, 23.999)), Ok(true));
        assert_eq!(rect.contains(Point::new(15.0, 22.0)), Ok(false));
        assert_eq!(rect.contains(Point::new(12.0, 24.0)), Ok(false));
        assert_eq!(rect.contains(Point::new(9.999, 22.0)), Ok(false));
    }

    #[test]
    fn edge_touching_rectangles_do_not_intersect() {
        let left = Rect::new(Point::new(0.0, 0.0), Size::new(10.0, 10.0));
        let touching = Rect::new(Point::new(10.0, 0.0), Size::new(5.0, 10.0));
        let overlapping = Rect::new(Point::new(9.0, 9.0), Size::new(3.0, 3.0));
        assert_eq!(left.intersects(touching), Ok(false));
        assert_eq!(left.intersects(overlapping), Ok(true));
    }

    #[test]
    fn rejects_overflowed_edges_and_non_finite_points() {
        let overflowing = Rect::new(Point::new(f32::MAX, 0.0), Size::new(f32::MAX, 1.0));
        assert_eq!(
            overflowing.validate(),
            Err(GeometryError::ArithmeticOverflow)
        );

        let valid = Rect::new(Point::ZERO, Size::new(1.0, 1.0));
        assert_eq!(
            valid.contains(Point::new(f32::NAN, 0.0)),
            Err(GeometryError::NonFinite)
        );
    }

    #[test]
    fn rejects_positive_dimensions_whose_edges_collapse_at_large_origins() {
        // At 2^24, adjacent f32 values are two units apart; adding one rounds
        // back to the original coordinate rather than creating a distinct edge.
        let horizontal = Rect::new(Point::new(16_777_216.0, 0.0), Size::new(1.0, 10.0));
        assert_eq!(
            horizontal.checked_right(),
            Err(GeometryError::NonPositiveDimension)
        );
        assert_eq!(
            horizontal.validate(),
            Err(GeometryError::NonPositiveDimension)
        );
        assert_eq!(
            horizontal.validate_non_empty(),
            Err(GeometryError::NonPositiveDimension)
        );
        assert_eq!(
            horizontal.contains(Point::new(16_777_216.0, 5.0)),
            Err(GeometryError::NonPositiveDimension)
        );
        assert_eq!(
            horizontal.intersects(Rect::new(
                Point::new(16_777_216.0, 0.0),
                Size::new(4.0, 10.0),
            )),
            Err(GeometryError::NonPositiveDimension)
        );

        let vertical = Rect::new(Point::new(0.0, 16_777_216.0), Size::new(10.0, 1.0));
        assert_eq!(
            vertical.checked_bottom(),
            Err(GeometryError::NonPositiveDimension)
        );
        assert_eq!(
            vertical.validate(),
            Err(GeometryError::NonPositiveDimension)
        );
    }

    #[test]
    fn zero_area_rect_remains_valid_even_at_large_origins() {
        let empty = Rect::new(Point::new(16_777_216.0, 16_777_216.0), Size::ZERO);
        assert_eq!(empty.validate(), Ok(()));
        assert_eq!(empty.contains(empty.origin), Ok(false));
        assert_eq!(
            empty.validate_non_empty(),
            Err(GeometryError::NonPositiveDimension)
        );
    }

    #[test]
    fn zero_area_rect_is_empty_but_not_invalid() {
        let empty = Rect::new(Point::ZERO, Size::ZERO);
        assert_eq!(empty.validate(), Ok(()));
        assert_eq!(empty.contains(Point::ZERO), Ok(false));
        assert_eq!(
            empty.validate_non_empty(),
            Err(GeometryError::NonPositiveDimension)
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn rect_round_trips_through_serde() {
        let rect = Rect::new(Point::new(5.0, 7.0), Size::new(100.0, 60.0));
        let encoded = serde_json::to_string(&rect).unwrap();
        assert_eq!(serde_json::from_str::<Rect>(&encoded).unwrap(), rect);
    }
}
