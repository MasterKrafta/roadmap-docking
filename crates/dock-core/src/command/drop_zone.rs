/// Geometric zone used to describe a docking target.
///
/// Whether a zone is valid for a particular target is checked when resolving
/// or applying a command; for example, Photoshop groups cannot split left/right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DropZone {
    Center,
    Top,
    Bottom,
    Left,
    Right,
}
