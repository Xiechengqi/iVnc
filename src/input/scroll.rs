//! High precision scroll input, in browser logical pixels (not encoded pixels).

use smithay::{
    backend::input::{Axis, AxisSource},
    input::pointer::AxisFrame,
};

pub const MAX_SCROLL_PIXELS: f64 = 32768.0;
// The conventional Wayland wheel notch is 10 units, or 120 Chromium pixels.
const PIXELS_PER_AXIS_UNIT: f64 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollDelta {
    x: f64,
    y: f64,
}

impl ScrollDelta {
    pub fn from_pixels(x: f64, y: f64) -> Option<Self> {
        [x, y]
            .iter()
            .all(|v| v.is_finite() && v.abs() <= MAX_SCROLL_PIXELS)
            .then_some(Self { x, y })
    }

    pub fn axis_frame(self, time: u32) -> AxisFrame {
        // WheelEvent cannot reliably distinguish a wheel from a touchpad.
        // Continuous supplies smooth axis values without Finger's stop/inertia
        // requirements, or inventing discrete notches from sample magnitude.
        AxisFrame::new(time)
            .source(AxisSource::Continuous)
            .value(Axis::Horizontal, self.x / PIXELS_PER_AXIS_UNIT)
            .value(Axis::Vertical, self.y / PIXELS_PER_AXIS_UNIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractional_scroll_is_continuous_and_uses_the_wheel_calibration() {
        let frame = ScrollDelta::from_pixels(0.25, -120.0)
            .unwrap()
            .axis_frame(42);
        assert_eq!(frame.source, Some(AxisSource::Continuous));
        assert_eq!(frame.axis.0, 0.25 / 12.0);
        assert_eq!(frame.axis.1, -10.0);
        assert_eq!(frame.v120, None);
        assert_eq!(frame.time, 42);
    }

    #[test]
    fn rejects_non_finite_and_excessive_deltas() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 32768.01] {
            assert!(ScrollDelta::from_pixels(value, 0.0).is_none());
            assert!(ScrollDelta::from_pixels(0.0, -value).is_none());
        }
        assert!(ScrollDelta::from_pixels(-MAX_SCROLL_PIXELS, MAX_SCROLL_PIXELS).is_some());
    }
}
