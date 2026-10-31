//! Theme boundary for the docking adapter.
//!
//! The first integration keeps using the application's shared UI theme. This
//! alias gives DockHost and downstream code a stable dock-specific name while
//! dock-only colors, typography, radii, and icon tokens are extracted later.
pub use ui::Theme as DockTheme;
