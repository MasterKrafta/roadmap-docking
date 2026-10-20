use std::{fmt, rc::Rc};

use super::PanelMenuItem;

/// Provider for menu actions specific to one panel.
#[derive(Clone)]
pub struct PanelMenuSource(Rc<dyn Fn() -> Vec<PanelMenuItem>>);

impl PanelMenuSource {
    pub fn new<F>(source: F) -> Self
    where
        F: Fn() -> Vec<PanelMenuItem> + 'static,
    {
        Self(Rc::new(source))
    }

    pub fn empty() -> Self {
        Self::new(Vec::new)
    }

    pub fn items(&self) -> Vec<PanelMenuItem> {
        (self.0)()
    }
}

impl fmt::Debug for PanelMenuSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PanelMenuSource(..)")
    }
}
