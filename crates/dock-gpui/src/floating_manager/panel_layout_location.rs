use dock_core::FloatingGroupId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelLayoutLocation {
    Main,
    Floating(FloatingGroupId),
}
