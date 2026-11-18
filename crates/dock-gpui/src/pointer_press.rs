use super::pointer_press_target::PointerPressTarget;

/// A click candidate derived from one Core hit-test result and retained until
/// pointer-up. The release hit is recomputed; HitResult itself is never cached.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PointerPress {
    pub(super) target: PointerPressTarget,
    pub(super) layout_version: u64,
}
