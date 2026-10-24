mod commands;
mod controller;
mod host_impl;

pub(crate) use commands::{activate_tab, collapse_group};
pub(crate) use controller::DockController;
pub(crate) use host_impl::WindowHost;
