use dock_core::{
    ComputeRectsError, DropSource, FloatingGroupId, GeometryError, HitResult, LayoutRects, NodeId,
    Point, Rect,
};
use gpui::WindowId;

use super::{
    DockController, FloatingGeometryCache, FloatingRectCacheKey, RectCacheKey, ResolverCacheKey,
};

impl DockController {
    pub(crate) fn floating_column_viewport_origin_y_offset(
        &self,
        window_id: WindowId,
        group: FloatingGroupId,
        column: NodeId,
    ) -> Option<f32> {
        self.floating_column_viewports
            .get(&window_id)
            .filter(|(owner, current_column, _, _, _)| {
                *owner == group && *current_column == column
            })
            .map(|(_, _, _, _, origin_y)| *origin_y)
    }

    /// Updates the native viewport used to lay out one Floating Column.
    pub(crate) fn set_floating_column_viewport(
        &mut self,
        window_id: WindowId,
        group: FloatingGroupId,
        column: NodeId,
        width: f32,
        height: f32,
        origin_y_offset: f32,
    ) -> bool {
        if !width.is_finite()
            || width <= 0.0
            || !height.is_finite()
            || height <= 0.0
            || !origin_y_offset.is_finite()
        {
            return false;
        }
        let unchanged = self
            .floating_column_viewports
            .get(&window_id)
            .is_some_and(
                |(current_group, current_column, current_width, current_height, current_origin_y)| {
                    *current_group == group
                        && *current_column == column
                        && (*current_width - width).abs() <= 0.5
                        && (*current_height - height).abs() <= 0.5
                        && (*current_origin_y - origin_y_offset).abs() <= 0.5
                },
            );
        if unchanged {
            return false;
        }

        self.floating_column_viewports
            .insert(window_id, (group, column, width, height, origin_y_offset));
        self.floating_rects.remove(&window_id);
        self.drag.clear_resolver();
        true
    }

    /// Returns the cached geometry snapshot, computing it only when its key
    /// differs from the key of the current snapshot.
    pub fn geometry(&mut self) -> Result<&LayoutRects, ComputeRectsError> {
        self.ensure_geometry()?;
        self.rects
            .as_ref()
            .ok_or(ComputeRectsError::InvalidLayoutState)
    }

    /// Resolves one pointer event against the current geometry snapshot.
    ///
    /// The semantic result is returned by value and is never retained as
    /// controller state; callers must request a fresh hit for each event.
    pub fn hit(&mut self, point: Point) -> Result<HitResult, ComputeRectsError> {
        self.ensure_geometry()?;
        let rects = self
            .rects
            .as_ref()
            .ok_or(ComputeRectsError::InvalidLayoutState)?;
        self.layout.hit_test(point, rects, &self.metrics)
    }

    /// Backwards-compatible name for callers that predate the event-scoped hit API.
    pub fn hit_test(&mut self, point: Point) -> Result<HitResult, ComputeRectsError> {
        self.hit(point)
    }

    /// Returns the cached local geometry for one floating composition in one GPUI window.
    ///
    /// The cache is keyed by both WindowId and FloatingGroupId. This matters when
    /// FloatingWindowManager reuses a hidden native Floating host for another group.
    pub fn floating_geometry(
        &mut self,
        window_id: WindowId,
        group: FloatingGroupId,
    ) -> Result<&LayoutRects, ComputeRectsError> {
        self.ensure_floating_geometry(window_id, group)?;
        self.floating_rects
            .get(&window_id)
            .map(|cache| &cache.rects)
            .ok_or(ComputeRectsError::InvalidLayoutState)
    }

    /// Resolves a pointer against the local geometry owned by a floating window.
    pub fn hit_floating_window(
        &mut self,
        window_id: WindowId,
        group: FloatingGroupId,
        point: Point,
    ) -> Result<HitResult, ComputeRectsError> {
        self.ensure_floating_geometry(window_id, group)?;
        let cache = self
            .floating_rects
            .get(&window_id)
            .ok_or(ComputeRectsError::InvalidLayoutState)?;
        self.layout.hit_test(point, &cache.rects, &self.metrics)
    }

    /// Builds or reuses a drop resolver from this floating window's local snapshot.
    ///
    /// The key includes the GPUI window and Core group identities, so a resolver
    /// prepared for the main window or a previously hosted group is never reused.
    pub fn prepare_floating_window_drop_resolver(
        &mut self,
        window_id: WindowId,
        group: FloatingGroupId,
        source: DropSource,
    ) -> Result<&dock_core::DropZoneResolver, ComputeRectsError> {
        if !matches!(
            self.drag.session().state(),
            dock_core::DragState::Dragging { .. }
        ) || self.drag.session().handle() == Some(dock_core::DragHandle::FloatingTitle) {
            return Err(ComputeRectsError::InvalidLayoutState);
        }

        self.ensure_floating_geometry(window_id, group)?;
        let key = self
            .floating_rects
            .get(&window_id)
            .map(|cache| cache.key)
            .ok_or(ComputeRectsError::InvalidLayoutState)?;
        let resolver_is_current = self
            .drag
            .floating_resolver
            .as_ref()
            .is_some_and(|(cached_key, cached_source, _)| {
                *cached_key == key && *cached_source == source
            });

        if !resolver_is_current {
            let resolver = {
                let cache = self
                    .floating_rects
                    .get(&window_id)
                    .ok_or(ComputeRectsError::InvalidLayoutState)?;
                self.layout
                    .prepare_drop_resolver(source, &cache.rects, &self.metrics)?
            };
            self.drag.set_floating_resolver(resolver, key, source);
        } else {
            self.drag.resolver_key = Some(ResolverCacheKey::Floating(key));
        }

        self.drag
            .resolver()
            .ok_or(ComputeRectsError::InvalidLayoutState)
    }

    /// Updates the identity of the main rendering window and invalidates its snapshot.
    pub(crate) fn set_main_window_id(&mut self, window_id: WindowId) {
        if self.main_window_id == Some(window_id) {
            return;
        }
        if self.has_active_pointer_interaction()
            && let Err(error) = self.cancel_pointer_interaction()
        {
            tracing::warn!(%error, "could not cancel dock interaction before main window replacement");
        }
        self.main_window_id = Some(window_id);
        self.rects = None;
        self.rects_key = None;
        self.drag.clear_resolver();
    }

    /// Builds or reuses a resolver for an active docking drag after its threshold.
    /// FloatingTitle is a window move and is rejected without preparing dock zones.
    pub fn prepare_drop_resolver(
        &mut self,
        source: DropSource,
    ) -> Result<&dock_core::DropZoneResolver, ComputeRectsError> {
        if !matches!(
            self.drag.session().state(),
            dock_core::DragState::Dragging { .. }
        ) || self.drag.session().handle() == Some(dock_core::DragHandle::FloatingTitle)
        {
            return Err(ComputeRectsError::InvalidLayoutState);
        }
        self.ensure_geometry()?;
        let key = self.rect_cache_key();
        let resolver_is_current = self
            .drag
            .main_resolver
            .as_ref()
            .is_some_and(|(cached_key, cached_source, _)| {
                *cached_key == key && *cached_source == source
            });

        if !resolver_is_current {
            let resolver = self.layout.prepare_drop_resolver(
                source,
                self.rects
                    .as_ref()
                    .ok_or(ComputeRectsError::InvalidLayoutState)?,
                &self.metrics,
            )?;
            self.drag.set_main_resolver(resolver, key, source);
        } else {
            self.drag.resolver_key = Some(ResolverCacheKey::Main(key));
        }

        self.drag
            .resolver()
            .ok_or(ComputeRectsError::InvalidLayoutState)
    }

    /// Returns the resolver only when it belongs to the current main-window snapshot.
    pub(crate) fn main_drop_resolver(&self) -> Option<&dock_core::DropZoneResolver> {
        let key = ResolverCacheKey::Main(self.rect_cache_key());
        (self.drag.resolver_key == Some(key))
            .then(|| self.drag.resolver())
            .flatten()
    }

    /// Returns main-window preview data only while its resolver owns the active drag.
    pub(crate) fn main_drag_preview(&self) -> Option<dock_core::DragPreview> {
        self.main_drop_resolver()?;
        self.drag.session().preview()
    }

    /// Returns the resolver only for the exact floating window/group cache entry.
    pub(crate) fn floating_window_drop_resolver(
        &self,
        window_id: WindowId,
        group: FloatingGroupId,
    ) -> Option<&dock_core::DropZoneResolver> {
        let expected = self.floating_geometry_cache_key(window_id, group);
        let cache_is_current = self
            .floating_rects
            .get(&window_id)
            .is_some_and(|cache| cache.key == expected);
        if !cache_is_current
            || self.drag.resolver_key != Some(ResolverCacheKey::Floating(expected))
        {
            return None;
        }
        self.drag.resolver()
    }

    /// Returns floating-window preview data only while its local resolver is current.
    pub(crate) fn floating_window_drag_preview(
        &self,
        window_id: WindowId,
        group: FloatingGroupId,
    ) -> Option<dock_core::DragPreview> {
        self.floating_window_drop_resolver(window_id, group)?;
        self.drag.session().preview()
    }

    /// Updates the viewport in logical pixels and invalidates adapter caches.
    pub fn set_viewport(&mut self, viewport: Rect) -> Result<bool, GeometryError> {
        viewport.validate_non_empty()?;
        if self.viewport == viewport {
            return Ok(false);
        }
        if let Err(error) = self.cancel_pointer_interaction() {
            tracing::warn!(%error, "could not cancel dock interaction before viewport change");
        }
        self.viewport = viewport;
        self.rects_key = None;
        self.drag.clear_resolver();
        Ok(true)
    }

    fn floating_geometry_cache_key(
        &self,
        window_id: WindowId,
        group: FloatingGroupId,
    ) -> FloatingRectCacheKey {
        let column_viewport = self
            .floating_column_viewports
            .get(&window_id)
            .copied()
            .filter(|(owner, _, _, _, _)| *owner == group);
        FloatingRectCacheKey {
            window_id,
            group,
            column: column_viewport.map(|(_, column, _, _, _)| column),
            column_width: column_viewport.map(|(_, _, width, _, _)| width),
            column_height: column_viewport.map(|(_, _, _, height, _)| height),
            column_origin_y_offset: column_viewport.map(|(_, _, _, _, origin_y)| origin_y),
            layout_version: self.layout.version(),
            metrics_revision: self.metrics_revision,
            chrome: self.layout.chrome(),
        }
    }

    fn ensure_floating_geometry(
        &mut self,
        window_id: WindowId,
        group: FloatingGroupId,
    ) -> Result<(), ComputeRectsError> {
        let key = self.floating_geometry_cache_key(window_id, group);
        if self
            .floating_rects
            .get(&window_id)
            .is_some_and(|cache| cache.key == key)
        {
            return Ok(());
        }

        let column_viewport = self
            .floating_column_viewports
            .get(&window_id)
            .copied()
            .filter(|(owner, _, _, _, _)| *owner == group);
        let rects = match column_viewport {
            Some((_, column, width, height, origin_y)) => self
                .layout
                .compute_floating_column_rects(
                    group,
                    column,
                    width,
                    height,
                    origin_y,
                    &self.metrics,
                ),
            None => self.layout.compute_floating_rects(group, &self.metrics),
        };
        let rects = match rects {
            Ok(rects) => rects,
            Err(error) => {
                self.floating_rects.remove(&window_id);
                return Err(error);
            }
        };
        self.floating_rects
            .insert(window_id, FloatingGeometryCache { key, rects });
        Ok(())
    }

    fn ensure_geometry(&mut self) -> Result<(), ComputeRectsError> {
        let key = self.rect_cache_key();
        if self.rects_key == Some(key) && self.rects.is_some() {
            return Ok(());
        }

        let rects = self.layout.compute_rects(self.viewport, &self.metrics)?;
        // Host-measured Canvas bounds are auxiliary state: this does not advance
        // Layout::version() and must not cause another geometry pass. Avoid even
        // calling the Core setter when the measured rectangle is unchanged.
        if self.layout.canvas().last_known_bounds() != rects.canvas {
            self.layout
                .update_canvas_bounds(rects.canvas)
                .map_err(ComputeRectsError::Geometry)?;
        }

        self.rects = Some(rects);
        self.rects_key = Some(key);
        Ok(())
    }

    fn rect_cache_key(&self) -> RectCacheKey {
        let flyout = self.layout.flyout();
        let flyout_width = flyout
            .and_then(|state| self.layout.panel_instance(state.panel))
            .and_then(|instance| self.layout.flyout_width(&instance.key));
        RectCacheKey {
            window_id: self.main_window_id,
            layout_version: self.layout.version(),
            viewport: self.viewport,
            metrics_revision: self.metrics_revision,
            chrome: self.layout.chrome(),
            flyout,
            flyout_width,
        }
    }

    /// Returns a read-only view of the currently cached snapshot for diagnostics.
    pub fn cached_geometry(&self) -> Option<&LayoutRects> {
        self.rects.as_ref()
    }

    pub fn cached_geometry_is_current(&self) -> bool {
        self.rects.is_some() && self.rects_key == Some(self.rect_cache_key())
    }
}
