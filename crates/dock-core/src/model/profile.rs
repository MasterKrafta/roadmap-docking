/// Validation rules are selected explicitly by profile; constraints are not global.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Profile {
    Photoshop,
}
