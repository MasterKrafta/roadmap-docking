use dock_core::{GeometryError, Point, Rect, Size};
use gpui::{Bounds, Pixels, Point as GpuiPoint};

/// GPUI layout coordinates are already logical pixels. This boundary converts
/// them once into dock-core values and deliberately does not reapply scale_factor.
pub(crate) fn point_from_gpui(point: GpuiPoint<Pixels>) -> Result<Point, GeometryError> {
    let point = Point::new(point.x.as_f32(), point.y.as_f32());
    point.validate()?;
    Ok(point)
}

/// Converts DockHost bounds into its host-local Core viewport and the window
/// origin required to map future pointer events into that same coordinate space.
pub(crate) fn host_geometry_from_gpui_bounds(
    bounds: Bounds<Pixels>,
) -> Result<(Rect, Point), GeometryError> {
    let window_origin = point_from_gpui(bounds.origin)?;
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
    point_from_gpui(point)?.checked_offset(-window_origin.x, -window_origin.y)
}

#[cfg(test)]
mod tests {
    use dock_core::{GeometryError, Point, Rect, Size};
    use gpui::{Bounds, Point as GpuiPoint, Size as GpuiSize, px};

    use super::{
        host_geometry_from_gpui_bounds, host_point_from_gpui_window, point_from_gpui,
    };

    #[test]
    fn gpui_point_converts_to_logical_core_pixels_once() {
        assert_eq!(
            point_from_gpui(GpuiPoint::new(px(12.5), px(34.25))),
            Ok(Point::new(12.5, 34.25))
        );
    }

    #[test]
    fn non_finite_pointer_coordinates_are_rejected() {
        assert_eq!(
            point_from_gpui(GpuiPoint::new(px(f32::NAN), px(0.0))),
            Err(GeometryError::NonFinite)
        );
        assert_eq!(
            point_from_gpui(GpuiPoint::new(px(0.0), px(f32::INFINITY))),
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
            host_geometry_from_gpui_bounds(bounds),
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
            assert!(host_geometry_from_gpui_bounds(bounds).is_err());
        }
    }

    #[test]
    fn window_pointer_is_translated_to_host_local_coordinates() {
        assert_eq!(
            host_point_from_gpui_window(
                GpuiPoint::new(px(118.0), px(232.0)),
                Point::new(18.0, 32.0)
            ),
            Ok(Point::new(100.0, 200.0))
        );
    }
}
