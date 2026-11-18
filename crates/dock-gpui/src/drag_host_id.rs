/// Runtime identity of one temporary drag-presentation window.
///
/// IDs are allocated by the adapter's DragHost manager. They are deliberately
/// independent from GPUI WindowId and Core FloatingGroupId.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DragHostId(u64);

impl DragHostId {
    /// Wraps an adapter-assigned runtime value.
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Returns the adapter-assigned runtime value.
    pub const fn get(self) -> u64 {
        self.0
    }
}
