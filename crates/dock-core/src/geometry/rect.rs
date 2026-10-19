use super::{Point, Size};
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect { pub origin: Point, pub size: Size }
impl Rect { pub const fn new(origin: Point, size: Size) -> Self { Self { origin, size } } }
