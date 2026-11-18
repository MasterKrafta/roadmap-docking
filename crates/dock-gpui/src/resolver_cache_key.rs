use super::{FloatingRectCacheKey, RectCacheKey};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ResolverCacheKey {
    Main(RectCacheKey),
    Floating(FloatingRectCacheKey),
}
