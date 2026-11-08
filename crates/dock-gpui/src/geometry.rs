use dock_core::{GeometryError, Point, Rect, Size};
use gpui::{Bounds, Pixels, Point as GpuiPoint};
use platform::{PhysicalPoint, PhysicalRect, PhysicalSize};

/// The single conversion boundary between GPUI logical pixels, DockCore
/// coordinates and native virtual-desktop physical pixels.
///
/// DockCore floating bounds intentionally combine physical screen origins
/// with logical client sizes. Keep that distinction explicit in this module.
pub(crate) struct DockGeometry;

impl DockGeometry {
    pub(crate) fn point_from_gpui(point: GpuiPoint<Pixels>) -> Result<Point, GeometryError> {
        let point = Point::new(point.x.as_f32(), point.y.as_f32());
        point.validate()?;
        Ok(point)
    }

    /// Converts DockHost bounds into its host-local Core viewport and the
    /// window origin needed to map future pointer events into that space.
    pub(crate) fn host_geometry_from_gpui_bounds(
        bounds: Bounds<Pixels>,
    ) -> Result<(Rect, Point), GeometryError> {
        let window_origin = Self::point_from_gpui(bounds.origin)?;
        let size = Size::new(bounds.size.width.as_f32(), bounds.size.height.as_f32());
        size.validate_positive()?;

        let viewport = Rect::new(Point::ZERO, size);
        viewport.validate_non_empty()?;
        Ok((viewport, window_origin))
    }

    /// Maps a GPUI window-space event to DockHost-local logical pixels.
    pub(crate) fn host_point_from_gpui_window(
        point: GpuiPoint<Pixels>,
        window_origin: Point,
    ) -> Result<Point, GeometryError> {
        window_origin.validate()?;
        Self::point_from_gpui(point)?.checked_offset(-window_origin.x, -window_origin.y)
    }

    /// Converts host-local logical coordinates to a physical desktop position.
    /// host_origin is the host's offset inside its GPUI window; use Point::ZERO
    /// for a floating host whose content begins at the native client origin.
    pub(crate) fn screen_position_from_host_local(
        local: Point,
        host_origin: Point,
        client_origin: PhysicalPoint,
        scale: f32,
    ) -> Option<PhysicalPoint> {
        if !valid_scale(scale) {
            return None;
        }
        let x = Self::physical_coordinate(host_origin.x + local.x, scale)?;
        let y = Self::physical_coordinate(host_origin.y + local.y, scale)?;
        Some(PhysicalPoint {
            x: client_origin.x.checked_add(x)?,
            y: client_origin.y.checked_add(y)?,
        })
    }

    /// Inverse of screen_position_from_host_local, including the DockHost
    /// offset inside the GPUI window.
    pub(crate) fn host_local_point_from_screen(
        screen: PhysicalPoint,
        client_origin: PhysicalPoint,
        host_origin: Point,
        scale: f32,
    ) -> Option<Point> {
        if !valid_scale(scale) {
            return None;
        }
        let x = (i64::from(screen.x) - i64::from(client_origin.x)) as f32 / scale
            - host_origin.x;
        let y = (i64::from(screen.y) - i64::from(client_origin.y)) as f32 / scale
            - host_origin.y;
        let point = Point::new(x, y);
        point.validate().ok()?;
        Some(point)
    }

    /// Converts a native outer-window rectangle and physical client size to
    /// the mixed Core floating-bounds representation.
    pub(crate) fn core_bounds_from_native(
        window_rect: PhysicalRect,
        client_size: PhysicalSize,
        scale: f32,
    ) -> Option<Rect> {
        if !client_size.is_valid() || !valid_scale(scale) {
            return None;
        }
        let bounds = Rect::new(
            Point::new(window_rect.left as f32, window_rect.top as f32),
            Size::new(
                client_size.width as f32 / scale,
                client_size.height as f32 / scale,
            ),
        );
        bounds.validate_non_empty().ok()?;
        Some(bounds)
    }

    pub(crate) fn physical_point_from_core(point: Point) -> Option<PhysicalPoint> {
        Some(PhysicalPoint {
            x: Self::integer_physical_coordinate(point.x)?,
            y: Self::integer_physical_coordinate(point.y)?,
        })
    }

    pub(crate) fn physical_offset_from_logical(
        delta: Point,
        scale: f32,
    ) -> Option<PhysicalPoint> {
        Some(PhysicalPoint {
            x: Self::physical_coordinate(delta.x, scale)?,
            y: Self::physical_coordinate(delta.y, scale)?,
        })
    }

    pub(crate) fn logical_delta_from_physical(
        delta: PhysicalPoint,
        scale: f32,
    ) -> Option<Point> {
        if !valid_scale(scale) {
            return None;
        }
        let point = Point::new(
            delta.x as f32 / scale,
            delta.y as f32 / scale,
        );
        point.validate().ok()?;
        Some(point)
    }

    pub(crate) fn physical_position_for_grab(
        cursor: PhysicalPoint,
        grab_offset: PhysicalPoint,
    ) -> PhysicalPoint {
        PhysicalPoint {
            x: cursor.x.saturating_sub(grab_offset.x),
            y: cursor.y.saturating_sub(grab_offset.y),
        }
    }

    pub(crate) fn physical_size_from_logical(
        size: Size,
        scale: f32,
    ) -> Option<PhysicalSize> {
        let size = PhysicalSize {
            width: Self::physical_coordinate(size.width, scale)?,
            height: Self::physical_coordinate(size.height, scale)?,
        };
        size.is_valid().then_some(size)
    }

    pub(crate) fn logical_size_from_physical(
        size: PhysicalSize,
        scale: f32,
    ) -> Option<Size> {
        if !size.is_valid() || !valid_scale(scale) {
            return None;
        }
        let size = Size::new(size.width as f32 / scale, size.height as f32 / scale);
        size.validate_positive().ok()?;
        Some(size)
    }

    pub(crate) fn logical_point_from_physical(
        point: PhysicalPoint,
        scale: f32,
    ) -> Option<Point> {
        if !valid_scale(scale) {
            return None;
        }
        let point = Point::new(point.x as f32 / scale, point.y as f32 / scale);
        point.validate().ok()?;
        Some(point)
    }

    pub(crate) fn physical_coordinate(value: f32, scale: f32) -> Option<i32> {
        if !valid_scale(scale) || !value.is_finite() {
            return None;
        }
        let rounded = (f64::from(value) * f64::from(scale)).round();
        if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
            return None;
        }
        Some(rounded as i32)
    }

    pub(crate) fn integer_physical_coordinate(value: f32) -> Option<i32> {
        if !value.is_finite() {
            return None;
        }
        let rounded = f64::from(value).round();
        if rounded < f64::from(i32::MIN) || rounded > f64::from(i32::MAX) {
            return None;
        }
        Some(rounded as i32)
    }
}

fn valid_scale(scale: f32) -> bool {
    scale.is_finite() && scale > 0.0
}

// Retain small, named GPUI adapters for callers that do not need to interact
// with native screen coordinates.
pub(crate) fn point_from_gpui(point: GpuiPoint<Pixels>) -> Result<Point, GeometryError> {
    DockGeometry::point_from_gpui(point)
}

pub(crate) fn host_geometry_from_gpui_bounds(
    bounds: Bounds<Pixels>,
) -> Result<(Rect, Point), GeometryError> {
    DockGeometry::host_geometry_from_gpui_bounds(bounds)
}

pub(crate) fn host_point_from_gpui_window(
    point: GpuiPoint<Pixels>,
    window_origin: Point,
) -> Result<Point, GeometryError> {
    DockGeometry::host_point_from_gpui_window(point, window_origin)
}

#[cfg(test)]
mod tests {
    use dock_core::{GeometryError, Point, Rect, Size};
    use gpui::{Bounds, Point as GpuiPoint, Size as GpuiSize, px};
    use platform::{PhysicalPoint, PhysicalRect, PhysicalSize};

    use super::DockGeometry;

    #[test]
    fn gpui_point_converts_to_logical_core_pixels_once() {
        assert_eq!(
            DockGeometry::point_from_gpui(GpuiPoint::new(px(12.5), px(34.25))),
            Ok(Point::new(12.5, 34.25))
        );
    }

    #[test]
    fn non_finite_pointer_coordinates_are_rejected() {
        assert_eq!(
            DockGeometry::point_from_gpui(GpuiPoint::new(px(f32::NAN), px(0.0))),
            Err(GeometryError::NonFinite)
        );
        assert_eq!(
            DockGeometry::point_from_gpui(GpuiPoint::new(px(0.0), px(f32::INFINITY))),
            Err(GeometryError::NonFinite)
        );
    }

    #[test]
    fn bounds_produce_host_viewport_and_window_origin() {
        let bounds = Bounds {
            origin: GpuiPoint::new(px(18.0), px(32.0)),
            size: GpuiSize::new(px(960.0), px(640.0)),
        };
        assert_eq!(
            DockGeometry::host_geometry_from_gpui_bounds(bounds),
            Ok((
                Rect::new(Point::ZERO, Size::new(960.0, 640.0)),
                Point::new(18.0, 32.0)
            ))
        );
    }

    #[test]
    fn invalid_bounds_are_rejected_before_core_receives_them() {
        for size in [
            GpuiSize::new(px(-1.0), px(640.0)),
            GpuiSize::new(px(0.0), px(640.0)),
            GpuiSize::new(px(960.0), px(f32::INFINITY)),
            GpuiSize::new(px(f32::NAN), px(640.0)),
        ] {
            let bounds = Bounds {
                origin: GpuiPoint::new(px(18.0), px(32.0)),
                size,
            };
            assert!(DockGeometry::host_geometry_from_gpui_bounds(bounds).is_err());
        }
    }

    #[test]
    fn window_pointer_is_translated_to_host_local_coordinates() {
        assert_eq!(
            DockGeometry::host_point_from_gpui_window(
                GpuiPoint::new(px(118.0), px(232.0)),
                Point::new(18.0, 32.0)
            ),
            Ok(Point::new(100.0, 200.0))
        );
    }

    #[test]
    fn main_host_coordinates_round_trip_at_fractional_monitor_dpi() {
        let client_origin = PhysicalPoint { x: -1920, y: 120 };
        let host_origin = Point::new(16.0, 24.0);
        let local = Point::new(113.6, 77.0);
        let scale = 1.25;

        let screen = DockGeometry::screen_position_from_host_local(
            local,
            host_origin,
            client_origin,
            scale,
        )
        .unwrap();
        let restored = DockGeometry::host_local_point_from_screen(
            screen,
            client_origin,
            host_origin,
            scale,
        )
        .unwrap();

        assert!((restored.x - local.x).abs() <= 0.25);
        assert!((restored.y - local.y).abs() <= 0.25);
    }

    #[test]
    fn physical_and_logical_sizes_convert_using_the_destination_scale() {
        let logical = Size::new(640.0, 480.0);
        let physical = DockGeometry::physical_size_from_logical(logical, 1.5).unwrap();
        assert_eq!(physical, PhysicalSize { width: 960, height: 720 });
        assert_eq!(
            DockGeometry::logical_size_from_physical(physical, 1.5),
            Some(logical)
        );
    }

    #[test]
    fn native_client_bounds_preserve_physical_origin_and_logical_size() {
        let native = PhysicalRect {
            left: -1920,
            top: 120,
            right: -960,
            bottom: 840,
        };
        assert_eq!(
            DockGeometry::core_bounds_from_native(
                native,
                PhysicalSize { width: 960, height: 720 },
                1.5,
            ),
            Some(Rect::new(
                Point::new(-1920.0, 120.0),
                Size::new(640.0, 480.0),
            ))
        );
    }

    #[test]
    fn conversion_rejects_invalid_scale_and_overflow() {
        assert!(
            DockGeometry::screen_position_from_host_local(
                Point::new(1.0, 1.0),
                Point::ZERO,
                PhysicalPoint::default(),
                0.0,
            )
            .is_none()
        );
        assert!(
            DockGeometry::logical_delta_from_physical(
                PhysicalPoint::default(),
                f32::NAN,
            )
            .is_none()
        );
        assert!(DockGeometry::physical_coordinate(f32::MAX, 2.0).is_none());
    }
}
