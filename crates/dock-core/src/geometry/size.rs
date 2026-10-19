#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Size { pub width: f32, pub height: f32 }
impl Size { pub const fn new(width: f32, height: f32) -> Self { Self { width, height } } }
