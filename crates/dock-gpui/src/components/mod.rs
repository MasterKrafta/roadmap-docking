//! Stateless GPUI presentation components for dock-core geometry snapshots.

pub mod column;
pub mod context_menu;
pub mod group;
pub mod main_dock;
pub mod panel_host;
pub mod rail;
pub mod splitter;
pub mod tab_overflow;
pub mod tab_strip;

pub use column::ColumnView;
pub use group::GroupView;
pub use main_dock::MainDockView;
pub use panel_host::PanelHost;
pub use rail::RailView;
pub use tab_overflow::TabOverflow;
pub use splitter::SplitterView;
pub use tab_strip::{TabHeaderView, TabStripView};

pub(crate) use context_menu::GroupMenuToggle;
pub(crate) use tab_overflow::TabOverflowToggle;
pub(crate) use tab_strip::collect_group_presentations;
