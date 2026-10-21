use crate::{GeometryError, Point, Size};

/// An axis-aligned rectangle in logical pixels.
///
/// The top and left edges are inclusive; the right and bottom edges are
/// exclusive: [x, x + width) × [y, y + height).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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

    /// Validates finite coordinates, non-negative dimensions, and finite edges.
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
        Ok(right)
    }

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
        Ok(bottom)
    }

    /// Returns whether the point lies inside the half-open rectangle.
    pub fn contains(self, point: Point) -> Result<bool, GeometryError> {
        self.validate()?;
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
        assert_eq!(overflowing.validate(), Err(GeometryError::ArithmeticOverflow));

        let valid = Rect::new(Point::ZERO, Size::new(1.0, 1.0));
        assert_eq!(
            valid.contains(Point::new(f32::NAN, 0.0)),
            Err(GeometryError::NonFinite)
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
}
