/// Checks whether the active drag has reached the point where content must move to DragHost.
/// PanelTab and GroupGrip leave their source zones; FlyoutPanel and DockArea start at threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MainDragHostKind {
    Panel,
    Group,
    DockArea,
}
