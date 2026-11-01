use dock_core::{
    ComputeRectsError, DockMetrics, DropSource, FloatingGroupId, GeometryError, HitResult,
    LayoutRects, Point, Rect,
};
use gpui::WindowId;

use super::{
    DockController, DockControllerConfigError, FloatingGeometryCache, FloatingRectCacheKey,
    RectCacheKey, ResolverCacheKey,
};

impl DockController {
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
    /// FloatingWindowManager reuses a hidden PopUp window for another group.
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
        ) || self.drag.session().handle() == Some(dock_core::DragHandle::FloatingTitle)
        {
            return Err(ComputeRectsError::InvalidLayoutState);
        }

        self.ensure_floating_geometry(window_id, group)?;
        let key = self
            .floating_rects
            .get(&window_id)
            .map(|cache| cache.key)
            .ok_or(ComputeRectsError::InvalidLayoutState)?;
        let resolver_key = ResolverCacheKey::Floating(key);
        let resolver_is_current = self.drag.resolver.is_some()
            && self.drag.resolver_key == Some(resolver_key)
            && self.drag.resolver_source == Some(source);

        if !resolver_is_current {
            let resolver = {
                let cache = self
                    .floating_rects
                    .get(&window_id)
                    .ok_or(ComputeRectsError::InvalidLayoutState)?;
                self.layout
                    .prepare_drop_resolver(source, &cache.rects, &self.metrics)?
            };
            self.drag.set_resolver(resolver, resolver_key, source);
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
        let key = ResolverCacheKey::Main(self.rect_cache_key());
        let resolver_is_current = self.drag.resolver.is_some()
            && self.drag.resolver_key == Some(key)
            && self.drag.resolver_source == Some(source);

        if !resolver_is_current {
            let resolver = self.layout.prepare_drop_resolver(
                source,
                self.rects
                    .as_ref()
                    .ok_or(ComputeRectsError::InvalidLayoutState)?,
                &self.metrics,
            )?;
            self.drag.set_resolver(resolver, key, source);
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
        let expected = FloatingRectCacheKey {
            window_id,
            group,
            layout_version: self.layout.version(),
            metrics_revision: self.metrics_revision,
            chrome: self.layout.chrome(),
        };
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

    /// Replaces metrics atomically. Existing layout must remain valid under them.
    pub fn set_metrics(&mut self, metrics: DockMetrics) -> Result<bool, DockControllerConfigError> {
        metrics
            .validate()
            .map_err(DockControllerConfigError::InvalidMetrics)?;
        if self.metrics == metrics {
            return Ok(false);
        }
        if let Err(error) = self.cancel_pointer_interaction() {
            tracing::warn!(%error, "could not cancel dock interaction before metrics change");
        }
        self.layout
            .validate(&metrics)
            .map_err(DockControllerConfigError::InvalidLayout)?;
        let next_revision = self
            .metrics_revision
            .checked_add(1)
            .ok_or(DockControllerConfigError::MetricsRevisionOverflow)?;
        let drag = super::DragController::new(&metrics)?;
        self.metrics = metrics;
        self.metrics_revision = next_revision;
        self.drag = drag;
        self.rects_key = None;
        self.floating_rects.clear();
        Ok(true)
    }

    fn ensure_floating_geometry(
        &mut self,
        window_id: WindowId,
        group: FloatingGroupId,
    ) -> Result<(), ComputeRectsError> {
        let key = FloatingRectCacheKey {
            window_id,
            group,
            layout_version: self.layout.version(),
            metrics_revision: self.metrics_revision,
            chrome: self.layout.chrome(),
        };
        if self
            .floating_rects
            .get(&window_id)
            .is_some_and(|cache| cache.key == key)
        {
            return Ok(());
        }

        let rects = match self.layout.compute_floating_rects(group, &self.metrics) {
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
