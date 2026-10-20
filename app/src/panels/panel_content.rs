use std::any::Any;

/// Type-erased panel content created by a descriptor's factory.
///
/// Content models are app-owned. A renderer such as the GPUI adapter can downcast
/// the model to its concrete type without introducing a GPUI dependency into dock-core.
pub trait PanelContent: Any {
    fn as_any(&self) -> &dyn Any;
}

impl<T: Any> PanelContent for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
