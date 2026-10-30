use crate::{
    ChromeMode, ColumnMode, DockSide, DropTarget, DropZone, FloatingGroupId, NodeId,
    PanelConstraints, PanelId, PanelKey, Point, Rect, Size,
};

/// A requested change to persistent docking state or an explicitly declared
/// transient UI state. Commands contain intent; execution belongs to Layout::apply.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DockCommand {
    /// Open a panel instance at an explicit destination, or use its last
    /// runtime dock location and then the profile default when target is None.
    OpenPanel {
        key: PanelKey,
        constraints: PanelConstraints,
        target: Option<DropTarget>,
    },
    /// Close one panel instance.
    ClosePanel { panel: PanelId },
    /// Close every panel in a group.
    CloseGroup { group: NodeId },
    /// Make the given panel the active tab in its group.
    ActivateTab { group: NodeId, panel: PanelId },
    /// Move an existing tab to an exact index in the same group.
    ReorderTab {
        group: NodeId,
        panel: PanelId,
        to_index: usize,
    },
    /// Move a panel to an explicit drop target.
    MovePanel { panel: PanelId, target: DropTarget },
    /// Complete a panel drag initiated from the currently open flyout.
    ///
    /// The command performs the same docking move as MovePanel, then closes that
    /// flyout in the same transaction. Invalid drops leave the flyout untouched.
    MovePanelFromFlyout { panel: PanelId, target: DropTarget },
    /// Float a panel dragged from the currently open flyout.
    ///
    /// This is the FreeArea completion path for FlyoutPanel drag handles.
    FloatPanelFromFlyout { panel: PanelId, bounds: Rect },
    /// Move a group into a column at an exact group slot.
    ///
    /// This also expresses insertion into an icon rail: the destination
    /// column and insertion index are both explicit.
    MoveGroup {
        group: NodeId,
        column: NodeId,
        index: usize,
    },
    /// Move a whole Tabs group to a new main-dock column insertion slot.
    ///
    /// When the group is the only group in an existing column on the same side,
    /// the existing column is reordered instead of creating a duplicate column.
    MoveGroupToMainColumn {
        group: NodeId,
        side: DockSide,
        index: usize,
    },
    /// Merge a whole Tabs group into another Tabs group at an exact tab slot.
    /// The source group container is removed; its Panel nodes and instances survive.
    /// In the Photoshop profile, a successful merge also expands a collapsed target.
    MergeGroup {
        group: NodeId,
        target_group: NodeId,
        index: usize,
    },
    /// Place a group adjacent to another group, applying the requested split
    /// ratio. The zone determines the split orientation and ordering.
    SplitGroup {
        group: NodeId,
        target_group: NodeId,
        zone: DropZone,
        ratio: f32,
    },
    /// Move one boundary between adjacent groups by a signed logical-pixel delta.
    ///
    /// available_height is the height available to the groups after splitter
    /// gaps are removed; Core uses it to enforce minimum expanded heights and
    /// the fixed height of collapsed groups.
    ResizeGroups {
        column: NodeId,
        boundary_index: usize,
        delta: f32,
        available_height: f32,
    },
    /// Set a column's requested width in logical pixels at the canvas-facing
    /// outer edge of a main dock. Interior main columns use ResizeColumnBoundary.
    ResizeColumn { column: NodeId, width: f32 },
    /// Move a boundary between adjacent main columns by a signed logical-pixel
    /// delta. Boundary index zero is between column zero and column one.
    ResizeColumnBoundary {
        side: DockSide,
        boundary_index: usize,
        delta: f32,
    },
    /// Move a boundary between adjacent Column children of a horizontal Split(X).
    ///
    /// Unlike ResizeColumnBoundary, this targets a split node directly and can
    /// therefore resize columns inside a floating dock-tree composition.
    ResizeSplitColumnBoundary {
        split: NodeId,
        boundary_index: usize,
        delta: f32,
    },
    /// Reset group-height ratios in one column, sharing free height among
    /// expanded groups only. Collapsed groups keep their fixed collapsed height.
    ResetColumnRatios {
        column: NodeId,
        available_height: f32,
    },
    /// Reset horizontal width ratios for the main columns on one dock side.
    ResetMainColumnRatios { side: DockSide },
    /// Restore an exact previously captured group-ratio vector (undo/history).
    ///
    /// Interactive resizing should use ResizeGroups or ResetColumnRatios.
    RestoreGroupRatios { column: NodeId, ratios: Vec<f32> },
    /// Detach a Panel or Tabs node into a floating group at these bounds.
    Float { node: NodeId, bounds: Rect },
    /// Move a floating window by setting its new origin.
    MoveFloating {
        group: FloatingGroupId,
        origin: Point,
    },
    /// Update a floating window's origin and size atomically during an edge/corner resize.
    ///
    /// This preserves one-command resize semantics when resizing from the top or left edges.
    ResizeFloatingBounds {
        group: FloatingGroupId,
        bounds: Rect,
    },
    /// Resize a floating window by setting its requested size.
    ResizeFloating { group: FloatingGroupId, size: Size },
    /// Raise a floating group to the front of the stacking order.
    RaiseFloating { group: FloatingGroupId },
    /// Dock a floating group at an explicit destination.
    DockFloating {
        group: FloatingGroupId,
        target: DropTarget,
    },
    /// Close a floating window and remove every panel instance owned by its area.
    CloseFloatingGroup { group: FloatingGroupId },
    /// Collapse or expand a Tabs group while preserving its children, active tab,
    /// and its parent container's existing size/ratio state.
    ///
    /// UI adapters recognize double-clicks on a tab header or unused tab-strip
    /// space and submit this command through `Layout::apply`. Gesture recognition
    /// and view-only state changes do not belong to Core.
    CollapseGroup { group: NodeId, collapsed: bool },
    /// Change a column between expanded, labeled-rail, and icon-only modes.
    ///
    /// Mode changes affect presentation geometry only; the column/group/tab tree
    /// and active-tab selections remain intact.
    SetColumnMode { column: NodeId, mode: ColumnMode },
    /// Show a transient flyout for a specific panel represented by a rail icon.
    ///
    /// The panel is selected in flyout display state only; its owning Tabs
    /// node's active_index remains unchanged.
    OpenFlyout { column: NodeId, panel: PanelId },
    /// Explicitly close the current transient flyout, regardless of auto-collapse policy.
    CloseFlyout,
    /// Dismiss a flyout in response to a click outside or Escape.
    ///
    /// This is ignored when auto-collapse is disabled. Adapters should use this for
    /// passive dismissal and CloseFlyout for an explicit close action.
    DismissFlyout,
    /// Enable or disable click-away, Escape, and same-icon auto-collapse behavior.
    ///
    /// This is transient UI configuration and never creates an undo record.
    SetFlyoutAutoCollapse { enabled: bool },
    /// Remember a flyout's requested width for the selected panel instance.
    ///
    /// Width is clamped to that panel's declared constraints. The command is transient:
    /// it does not change the dock tree, layout version, or undo history.
    ResizeFlyout { panel: PanelId, width: f32 },
    /// Set visibility of application chrome without rewriting the dock tree.
    SetChrome { mode: ChromeMode },
}

#[cfg(test)]
mod tests {
    use super::DockCommand;
    use crate::{
        ChromeMode, ColumnMode, DockSide, DropTarget, DropZone, FloatingGroupId, NodeId,
        PanelConstraints, PanelId, PanelKey, Point, Rect, Size,
    };
    use slotmap::Key;

    #[test]
    fn every_command_can_be_constructed_without_ui_types() {
        let key = PanelKey::new("layers").unwrap();
        let constraints =
            PanelConstraints::new(Size::new(120.0, 80.0), Size::new(280.0, 220.0), None, false);
        let target = DropTarget::RailSlot {
            column: NodeId::null(),
            index: 1,
        };
        let commands = [
            DockCommand::OpenPanel {
                key,
                constraints,
                target: Some(target),
            },
            DockCommand::ClosePanel {
                panel: PanelId::null(),
            },
            DockCommand::CloseGroup {
                group: NodeId::null(),
            },
            DockCommand::ActivateTab {
                group: NodeId::null(),
                panel: PanelId::null(),
            },
            DockCommand::ReorderTab {
                group: NodeId::null(),
                panel: PanelId::null(),
                to_index: 0,
            },
            DockCommand::MovePanel {
                panel: PanelId::null(),
                target,
            },
            DockCommand::MovePanelFromFlyout {
                panel: PanelId::null(),
                target,
            },
            DockCommand::FloatPanelFromFlyout {
                panel: PanelId::null(),
                bounds: Rect::new(Point::ZERO, Size::new(400.0, 300.0)),
            },
            DockCommand::MoveGroup {
                group: NodeId::null(),
                column: NodeId::null(),
                index: 0,
            },
            DockCommand::MoveGroupToMainColumn {
                group: NodeId::null(),
                side: DockSide::Right,
                index: 0,
            },
            DockCommand::MergeGroup {
                group: NodeId::null(),
                target_group: NodeId::null(),
                index: 0,
            },
            DockCommand::SplitGroup {
                group: NodeId::null(),
                target_group: NodeId::null(),
                zone: DropZone::Bottom,
                ratio: 0.5,
            },
            DockCommand::ResizeGroups {
                column: NodeId::null(),
                boundary_index: 0,
                delta: 12.0,
                available_height: 480.0,
            },
            DockCommand::ResizeColumn {
                column: NodeId::null(),
                width: 280.0,
            },
            DockCommand::ResizeColumnBoundary {
                side: DockSide::Right,
                boundary_index: 0,
                delta: 12.0,
            },
            DockCommand::ResizeSplitColumnBoundary {
                split: NodeId::null(),
                boundary_index: 0,
                delta: 12.0,
            },
            DockCommand::ResetColumnRatios {
                column: NodeId::null(),
                available_height: 480.0,
            },
            DockCommand::ResetMainColumnRatios {
                side: DockSide::Right,
            },
            DockCommand::RestoreGroupRatios {
                column: NodeId::null(),
                ratios: vec![1.0],
            },
            DockCommand::Float {
                node: NodeId::null(),
                bounds: Rect::new(Point::ZERO, Size::new(400.0, 300.0)),
            },
            DockCommand::MoveFloating {
                group: FloatingGroupId::null(),
                origin: Point::ZERO,
            },
            DockCommand::ResizeFloating {
                group: FloatingGroupId::null(),
                size: Size::new(500.0, 400.0),
            },
            DockCommand::ResizeFloatingBounds {
                group: FloatingGroupId::null(),
                bounds: Rect::new(Point::new(10.0, 20.0), Size::new(500.0, 400.0)),
            },
            DockCommand::RaiseFloating {
                group: FloatingGroupId::null(),
            },
            DockCommand::DockFloating {
                group: FloatingGroupId::null(),
                target,
            },
            DockCommand::CloseFloatingGroup {
                group: FloatingGroupId::null(),
            },
            DockCommand::CollapseGroup {
                group: NodeId::null(),
                collapsed: true,
            },
            DockCommand::SetColumnMode {
                column: NodeId::null(),
                mode: ColumnMode::IconsOnly,
            },
            DockCommand::OpenFlyout {
                column: NodeId::null(),
                panel: PanelId::null(),
            },
            DockCommand::CloseFlyout,
            DockCommand::DismissFlyout,
            DockCommand::SetFlyoutAutoCollapse { enabled: true },
            DockCommand::ResizeFlyout {
                panel: PanelId::null(),
                width: 280.0,
            },
            DockCommand::SetChrome {
                mode: ChromeMode::AllHidden,
            },
        ];
        assert_eq!(commands.len(), 34);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip_preserves_a_docking_command() {
        let command = DockCommand::ResizeColumnBoundary {
            side: DockSide::Left,
            boundary_index: 1,
            delta: -16.0,
        };
        let encoded = serde_json::to_string(&command).unwrap();
        assert_eq!(
            serde_json::from_str::<DockCommand>(&encoded).unwrap(),
            command
        );
    }
}
