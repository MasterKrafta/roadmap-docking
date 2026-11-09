//! Runtime identity and role types for dock-related native windows.
//!
//! These types describe GPUI adapter windows only. A DragHost is a temporary
//! presentation surface; it is not a Core DockArea, FloatingGroup, or workspace
//! entity and must never be inserted into or serialized with Layout.
//!
//! The future DragHostManager is owned by DockHost as a sibling of
//! FloatingWindowManager. Both managers receive the same DockController entity;
//! neither manager owns or duplicates the Core Layout.

use dock_core::FloatingGroupId;

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

/// Describes the runtime purpose of a native window used by the dock adapter.
///
/// This is not a Core model type. In particular, `Dragging` identifies a
/// temporary transport/presentation window and does not imply that it owns a
/// dock tree or a `FloatingGroup`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DockWindowRole {
    /// The application's persistent main window.
    Main,
    /// A persistent native window hosting one Core floating composition.
    Floating {
        /// Core identity of the composition hosted by this window.
        group: FloatingGroupId,
    },
    /// A temporary window used only while a dock object is being dragged.
    Dragging {
        /// Runtime identity of the temporary drag host.
        id: DragHostId,
    },
}

#[cfg(test)]
mod tests {
    use dock_core::FloatingGroupId;
    use slotmap::Key as _;

    use super::{DockWindowRole, DragHostId};

    #[test]
    fn drag_host_id_is_a_distinct_runtime_identity() {
        let first = DragHostId::from_raw(1);
        let second = DragHostId::from_raw(2);

        assert_ne!(first, second);
        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
    }

    #[test]
    fn dock_window_roles_keep_dragging_separate_from_floating_groups() {
        let drag_id = DragHostId::from_raw(7);
        let dragging = DockWindowRole::Dragging { id: drag_id };

        assert_eq!(dragging, DockWindowRole::Dragging { id: drag_id });
        assert_ne!(
            dragging,
            DockWindowRole::Floating {
                group: FloatingGroupId::null(),
            }
        );
    }
}
