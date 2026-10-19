use crate::ids::PanelId;
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tabs { pub panels: Vec<PanelId>, pub active_index: usize }
