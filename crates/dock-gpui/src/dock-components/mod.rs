mod column;
mod composition;
mod dock_area;
mod group_presentation;

pub(crate) use composition::DockCompositionView;
pub(crate) use group_presentation::{collect_group_presentations, GroupPresentation};
pub(crate) use column::DockGroupView;

pub use column::{
    DockColumnView, DockTab, GroupDragArea, GroupGrip, GroupMenuButton, PanelContent,
};
pub use dock_area::{
    CollapseColumnButton, CloseDockAreaButton, DockAreaDragArea, DockAreaHeader, DockAreaView,
};
