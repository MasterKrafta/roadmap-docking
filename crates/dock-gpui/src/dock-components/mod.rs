mod composition;
mod group_presentation;

pub(crate) use composition::{DockCompositionView, DockGroupView};
pub(crate) use group_presentation::{
    active_panels_for_groups, collect_group_presentations, GroupPresentation,
    PanelTransferPresentation,
  };

pub use composition::{
    CloseCompositionButton, CollapseColumnButton, CompositionDragArea, CompositionHeader,
    DockColumnView, DockTab, GroupDragArea, GroupGrip, GroupMenuButton, PanelContent,
};
