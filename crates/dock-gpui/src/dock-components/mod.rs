mod composition;
mod group_presentation;

pub(crate) use composition::{DockCompositionView, DockGroupView};
pub(crate) use group_presentation::{collect_group_presentations, GroupPresentation};

pub use composition::{
    CloseCompositionButton, CollapseColumnButton, CompositionDragArea, CompositionHeader,
    DockColumnView, DockTab, GroupDragArea, GroupGrip, GroupMenuButton, PanelContent,
};
