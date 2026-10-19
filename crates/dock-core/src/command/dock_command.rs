use crate::geometry::Rect;
use crate::ids::{FloatingGroupId, NodeId, PanelId, PanelKey};
use crate::model::PanelConstraints;

use super::{ColumnRef, DropTarget, FloatSource, SplitSide};

/// Serializable request to make one atomic change to the docking layout.
///
/// This enum describes operations only. Transactional execution is delegated to
/// Layout::apply; no command mutates a layout by itself.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DockCommand {
    /// Open a closed panel, or activate its existing instance.
    OpenPanel {
        key: PanelKey,
        constraints: PanelConstraints,
        target: Option<DropTarget>,
    },
    /// Close a panel instance. The protected Canvas is not a panel and cannot be closed.
    ClosePanel {
        panel: PanelId,
    },
    /// Close all panels in one tab group.
    CloseGroup {
        group: NodeId,
    },
    /// Make the tab at the zero-based index active.
    ActivateTab {
        group: NodeId,
        index: usize,
    },
    /// Move a tab to another zero-based position in the same group.
    ReorderTab {
        group: NodeId,
        from: usize,
        to: usize,
    },
    /// Move a panel to the selected dock or floating target.
    MovePanel {
        panel: PanelId,
        target: DropTarget,
    },
    /// Split a panel out of a group. Top/bottom create a group in the same
    /// column; left/right create a neighboring column in the Photoshop profile.
    SplitGroup {
        panel: PanelId,
        group: NodeId,
        side: SplitSide,
    },
    /// Move the boundary between adjacent vertical groups in a column.
    ResizeGroups {
        column: ColumnRef,
        boundary: usize,
        delta: f32,
    },
    /// Set a column's width in logical pixels. For an outer boundary the Canvas
    /// receives the remaining space; for an inner boundary use ResizeColumnBoundary.
    ResizeColumn {
        column: ColumnRef,
        width: f32,
    },
    /// Atomically resize two adjacent columns while preserving their combined width.
    ResizeColumnBoundary {
        first: ColumnRef,
        second: ColumnRef,
        delta: f32,
    },
    /// Restore even height distribution for the groups in a column.
    ResetColumnRatios {
        column: ColumnRef,
    },
    /// Detach a panel or group into an in-window floating group.
    Float {
        source: FloatSource,
        rect: Rect,
    },
    /// Move an existing floating group within the main window.
    MoveFloating {
        floating: FloatingGroupId,
        rect: Rect,
    },
    /// Resize an existing floating group.
    ResizeFloating {
        floating: FloatingGroupId,
        rect: Rect,
    },
    /// Raise a floating group above other floating groups.
    RaiseFloating {
        floating: FloatingGroupId,
    },
    /// Dock a floating group at a resolved target.
    DockFloating {
        floating: FloatingGroupId,
        target: DropTarget,
    },
}

#[cfg(test)]
mod tests {
    use super::DockCommand;

    #[test]
    fn command_enum_is_debuggable_and_cloneable() {
        fn assert_traits<T: Clone + core::fmt::Debug + PartialEq>() {}
        assert_traits::<DockCommand>();
    }

    #[cfg(feature = "serde")]
    #[test]
    fn command_and_support_types_implement_serde() {
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<DockCommand>();
        assert_serde::<super::super::DropTarget>();
        assert_serde::<super::super::DropZone>();
        assert_serde::<super::super::CommandError>();
        assert_serde::<super::super::CommandOutcome>();
        assert_serde::<super::super::ColumnRef>();
        assert_serde::<super::super::FloatSource>();
        assert_serde::<super::super::SplitSide>();
    }
}
