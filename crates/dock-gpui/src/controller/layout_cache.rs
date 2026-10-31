use dock_core::{
    ComputeRectsError, DockMetrics, DropSource, GeometryError, HitResult, LayoutRects, Point, Rect,
};

use super::{DockController, DockControllerConfigError, RectCacheKey};

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
        Ok(true)
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
