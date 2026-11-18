use gpui::SharedString;

/// An icon reference supplied by the application for a panel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IconSource {
    /// A path resolved by the GPUI asset source.
    Asset(SharedString),
    /// The descriptor intentionally has no icon.
    None,
}
