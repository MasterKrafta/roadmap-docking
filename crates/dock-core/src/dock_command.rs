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
    /// Move a group into a column at an exact group slot.
    ///
    /// This also expresses insertion into an icon rail: the destination
    /// column and insertion index are both explicit.
    MoveGroup {
        group: NodeId,
        column: NodeId,
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
    /// Set a column's requested width in logical pixels.
    ResizeColumn { column: NodeId, width: f32 },
    /// Move a boundary between adjacent main columns by a signed logical-pixel
    /// delta. Boundary index zero is between column zero and column one.
    ResizeColumnBoundary {
        side: DockSide,
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
    SetGroupRatios { column: NodeId, ratios: Vec<f32> },
    /// Detach a Panel or Tabs node into a floating group at these bounds.
    Float { node: NodeId, bounds: Rect },
    /// Move a floating window by setting its new origin.
    MoveFloating { group: FloatingGroupId, origin: Point },
    /// Resize a floating window by setting its requested size.
    ResizeFloating { group: FloatingGroupId, size: Size },
    /// Raise a floating group to the front of the stacking order.
    RaiseFloating { group: FloatingGroupId },
    /// Dock a floating group at an explicit destination.
    DockFloating { group: FloatingGroupId, target: DropTarget },
    /// Expand or collapse a tab group.
    CollapseGroup { group: NodeId, collapsed: bool },
    /// Change a column between expanded and icon-only modes.
    SetColumnMode { column: NodeId, mode: ColumnMode },
    /// Show a transient flyout for a group belonging to an icon rail.
    OpenFlyout { column: NodeId, group: NodeId },
    /// Close the current transient flyout, if any.
    CloseFlyout,
    /// Set visibility of application chrome without rewriting the dock tree.
    SetChrome { mode: ChromeMode },
}

#[cfg(test)]
mod tests {
    use slotmap::Key;
    use super::DockCommand;
    use crate::{
        ChromeMode, ColumnMode, DockSide, DropTarget, DropZone, FloatingGroupId, NodeId,
        PanelConstraints, PanelId, PanelKey, Point, Rect, Size,
    };

    #[test]
    fn every_command_can_be_constructed_without_ui_types() {
        let key = PanelKey::new("layers").unwrap();
        let constraints = PanelConstraints::new(
            Size::new(120.0, 80.0),
            Size::new(280.0, 220.0),
            None,
            false,
        );
        let target = DropTarget::RailSlot { column: NodeId::null(), index: 1 };
        let commands = [
            DockCommand::OpenPanel { key, constraints, target: Some(target) },
            DockCommand::ClosePanel { panel: PanelId::null() },
            DockCommand::CloseGroup { group: NodeId::null() },
            DockCommand::ActivateTab { group: NodeId::null(), panel: PanelId::null() },
            DockCommand::ReorderTab { group: NodeId::null(), panel: PanelId::null(), to_index: 0 },
            DockCommand::MovePanel { panel: PanelId::null(), target },
            DockCommand::MoveGroup { group: NodeId::null(), column: NodeId::null(), index: 0 },
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
            DockCommand::ResizeColumn { column: NodeId::null(), width: 280.0 },
            DockCommand::ResizeColumnBoundary {
                side: DockSide::Right,
                boundary_index: 0,
                delta: 12.0,
            },
            DockCommand::ResetColumnRatios {
                column: NodeId::null(),
                available_height: 480.0,
            },
            DockCommand::ResetMainColumnRatios { side: DockSide::Right },
            DockCommand::SetGroupRatios { column: NodeId::null(), ratios: vec![1.0] },
            DockCommand::Float {
                node: NodeId::null(),
                bounds: Rect::new(Point::ZERO, Size::new(400.0, 300.0)),
            },
            DockCommand::MoveFloating { group: FloatingGroupId::null(), origin: Point::ZERO },
            DockCommand::ResizeFloating {
                group: FloatingGroupId::null(),
                size: Size::new(500.0, 400.0),
            },
            DockCommand::RaiseFloating { group: FloatingGroupId::null() },
            DockCommand::DockFloating { group: FloatingGroupId::null(), target },
            DockCommand::CollapseGroup { group: NodeId::null(), collapsed: true },
            DockCommand::SetColumnMode { column: NodeId::null(), mode: ColumnMode::IconsOnly },
            DockCommand::OpenFlyout { column: NodeId::null(), group: NodeId::null() },
            DockCommand::CloseFlyout,
            DockCommand::SetChrome { mode: ChromeMode::Hidden },
        ];
        assert_eq!(commands.len(), 24);
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
        assert_eq!(serde_json::from_str::<DockCommand>(&encoded).unwrap(), command);
    }
}
