//! Native platform integrations used by the docking system.
//!
//! Keep operating-system-specific code in this crate. Add another backend only
//! when the docking implementation needs it.

#[cfg(windows)]
pub mod windows;
