use dock_core::LayoutRects;

use super::FloatingRectCacheKey;

struct FloatingGeometryCache {
    pub(super) key: FloatingRectCacheKey,
    pub(super) rects: LayoutRects,
}
