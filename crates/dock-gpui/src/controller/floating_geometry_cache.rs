use dock_core::LayoutRects;

use super::FloatingRectCacheKey;

pub(super) struct FloatingGeometryCache {
    pub(super) key: FloatingRectCacheKey,
    pub(super) rects: LayoutRects,
}
