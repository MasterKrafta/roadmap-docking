use std::{fmt, rc::Rc};

use super::PanelContent;

/// Factory for panel-owned content, intentionally independent of any UI framework.
#[derive(Clone)]
pub struct PanelContentFactory(Rc<dyn Fn() -> Box<dyn PanelContent>>);

impl PanelContentFactory {
    pub fn new<F, C>(factory: F) -> Self
    where
        F: Fn() -> C + 'static,
        C: PanelContent + 'static,
    {
        Self(Rc::new(move || -> Box<dyn PanelContent> {
            Box::new(factory())
        }))
    }

    pub fn create(&self) -> Box<dyn PanelContent> {
        (self.0)()
    }
}

impl fmt::Debug for PanelContentFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PanelContentFactory(..)")
    }
}
