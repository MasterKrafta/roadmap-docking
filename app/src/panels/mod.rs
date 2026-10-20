mod allowed_area;
mod icon_resource;
mod panel_content;
mod panel_content_factory;
mod panel_descriptor;
mod panel_menu_item;
mod panel_menu_source;
mod panel_registry;
mod placeholder_panel_content;
mod registry_error;

pub use allowed_area::AllowedArea;
pub use icon_resource::IconResource;
pub use panel_content::PanelContent;
pub use panel_content_factory::PanelContentFactory;
pub use panel_descriptor::PanelDescriptor;
pub use panel_menu_item::PanelMenuItem;
pub use panel_menu_source::PanelMenuSource;
pub use panel_registry::PanelRegistry;
pub use placeholder_panel_content::PlaceholderPanelContent;
pub use registry_error::RegistryError;

use dock_core::PanelKey;
