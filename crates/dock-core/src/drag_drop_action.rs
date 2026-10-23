use crate::DockCommand;

/// A pure result of resolving a released drag request against a Layout.
///
/// `NoOp` means the source was dropped onto itself or its existing position;
/// callers must not submit a command for it. `Command` contains exactly one
/// command for the adapter to execute once.
#[derive(Clone, Debug, PartialEq)]
pub enum DragDropAction {
    NoOp,
    Command(DockCommand),
}
