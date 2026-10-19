use crate::geometry::Rect;
use crate::ids::PanelId;
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FloatingGroup { pub panels: Vec<PanelId>, pub active_index: usize, pub bounds: Rect, pub z_order: u64 }
