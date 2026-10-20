use super::{Split, Tabs};
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Node {
    Split(Split),
    Tabs(Tabs),
}
