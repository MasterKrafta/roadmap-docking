/// Workspace regions in which a panel descriptor permits placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AllowedArea {
    Dock,
    Floating,
    Rail,
}
