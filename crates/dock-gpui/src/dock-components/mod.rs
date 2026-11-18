#[path = "composition_header.rs"]
mod composition_header;
#[path = "group.rs"]
mod group;
#[path = "tab.rs"]
mod tab;
mod group_presentation;

pub(crate) use group::DockGroupView;
pub(crate) use group_presentation::{
    active_panels_for_groups, collect_group_presentations, GroupPresentation,
    PanelTransferPresentation,
};

pub use composition_header::{
    CloseCompositionButton, CollapseColumnButton, CompositionDragArea, CompositionHeader,
};
pub use group::{GroupDragArea, GroupGrip, GroupMenuButton, PanelContent};
pub use tab::DockTab;
