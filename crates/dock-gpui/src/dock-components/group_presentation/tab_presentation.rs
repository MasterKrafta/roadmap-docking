use dock_core::PanelId;
use gpui::SharedString;

#[derive(Clone)]
pub(crate) struct TabPresentation {
    pub panel: PanelId,
    pub title: SharedString,
    pub icon: Option<SharedString>,
    pub index: usize,
}
