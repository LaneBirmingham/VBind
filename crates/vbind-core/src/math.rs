//! Signal math shared by every mapping: deadzones, response curves, stick
//! mixers and conversions to virtual-device report units.
//!
//! All analog values are `f64` in `-1.0..=1.0` (centered axes) or `0.0..=1.0`
//! (one-way levers and triggers).

/// Clamp `v` into `lo..=hi`, mapping NaN to `lo`.
pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v.is_nan() {
        return lo;
    }
    v.max(lo).min(hi)
}

/// Normalize a DirectInput axis reading in `-32768..=32767` to `-1.0..=1.0`.
pub fn normalize_di_axis(raw: i32) -> f64 {
    let n = if raw < 0 {
        raw as f64 / 32768.0
    } else {
        raw as f64 / 32767.0
    };
    clamp(n, -1.0, 1.0)
}

/// Centered deadzone with rescaling: values within `dz` of center become 0 and
/// the remaining travel is stretched so full deflection still reaches 1.
/// `dz` is a fraction (`0.06` = 6%).
pub fn deadzone(v: f64, dz: f64) -> f64 {
    let dz = clamp(dz, 0.0, 0.95);
    let mag = v.abs().min(1.0);
    if mag <= dz {
        return 0.0;
    }
    ((mag - dz) / (1.0 - dz)).copysign(v)
}

/// One-way deadzone for levers and triggers in `0..=1`.
pub fn deadzone_one_way(v: f64, dz: f64) -> f64 {
    let dz = clamp(dz, 0.0, 0.95);
    let v = clamp(v, 0.0, 1.0);
    if v <= dz {
        return 0.0;
    }
    (v - dz) / (1.0 - dz)
}

/// Weighted power response curve on a magnitude in `0..=1`.
///
/// `power == 1` is linear. `weight` in `0..=1` blends a pure power curve
/// (`0`) with its mirrored inverse (`1`); `0.5` gives a smooth S-like arc.
pub fn weighted_arc(x: f64, power: f64, weight: f64) -> f64 {
    let x = clamp(x, 0.0, 1.0);
    if x == 0.0 || x == 1.0 || power == 1.0 {
        return x;
    }
    let weight = clamp(weight, 0.0, 1.0);
    (1.0 - weight) * x.powf(power) + weight * (1.0 - (1.0 - x).powf(1.0 / power))
}

/// Apply a symmetric response curve to a centered value.
/// `full_output` (`0.5..=1`) is the deflection at which output reaches 100%.
pub fn curve(v: f64, power: f64, weight: f64, full_output: f64) -> f64 {
    let full = clamp(full_output, 0.05, 1.0);
    let mag = (v.abs() / full).min(1.0);
    clamp(weighted_arc(mag, power, weight), 0.0, 1.0).copysign(v)
}

/// Two-stick ("HOSAS"/podracer) thrust.
///
/// With `preserve_thrust` (the default in the original bridge):
/// - both sticks forward: the stronger stick sets thrust, so steering by
///   holding one stick back a little costs no speed;
/// - one forward, one back: the backward stick scrubs thrust in proportion
///   (`R = 1`: `L = -0.25` gives 0.75, `L = -1` gives 0);
/// - both back: their average, so both fully back is full brake/reverse.
///
/// Without it, thrust is the plain average of both sticks.
pub fn hosas_drive(left: f64, right: f64, preserve_thrust: bool) -> f64 {
    if !preserve_thrust {
        return clamp((left + right) / 2.0, -1.0, 1.0);
    }
    if left >= 0.0 && right >= 0.0 {
        return left.max(right);
    }
    if left <= 0.0 && right <= 0.0 {
        return clamp((left + right) / 2.0, -1.0, 0.0);
    }
    if left > 0.0 && right < 0.0 {
        return clamp(left * (1.0 - right.abs()), 0.0, 1.0);
    }
    if right > 0.0 && left < 0.0 {
        return clamp(right * (1.0 - left.abs()), 0.0, 1.0);
    }
    0.0
}

/// Hat directions in hundredths of a degree, clockwise from up, as reported
/// by DirectInput.
pub mod hat {
    pub const UP: u32 = 0;
    pub const UP_RIGHT: u32 = 4500;
    pub const RIGHT: u32 = 9000;
    pub const DOWN_RIGHT: u32 = 13500;
    pub const DOWN: u32 = 18000;
    pub const DOWN_LEFT: u32 = 22500;
    pub const LEFT: u32 = 27000;
    pub const UP_LEFT: u32 = 31500;
    /// Default match tolerance: 23 degrees, so a diagonal does not count as
    /// either neighbouring cardinal direction.
    pub const DEFAULT_TOLERANCE: u32 = 2300;
}

/// Smallest angular distance between two hat angles (hundredths of degrees).
pub fn hat_angle_distance(a: u32, b: u32) -> u32 {
    let d = (a as i64 - b as i64).unsigned_abs() as u32 % 36000;
    if d > 18000 { 36000 - d } else { d }
}

/// Whether a hat reading (`None` = centered) points at `target` within
/// `tolerance` hundredths of a degree.
pub fn hat_matches(reading: Option<u32>, target: u32, tolerance: u32) -> bool {
    match reading {
        Some(angle) => hat_angle_distance(angle, target) <= tolerance,
        None => false,
    }
}

/// Convert `0..=1` to an Xbox trigger byte.
pub fn to_trigger(v: f64) -> u8 {
    (clamp(v, 0.0, 1.0) * 255.0).round() as u8
}

/// Convert `-1..=1` to an Xbox thumbstick axis.
pub fn to_stick(v: f64) -> i16 {
    let v = clamp(v, -1.0, 1.0);
    if v < 0.0 {
        (v * 32768.0).round() as i16
    } else {
        (v * 32767.0).round() as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn deadzone_zeroes_center_and_rescales() {
        assert_eq!(deadzone(0.05, 0.06), 0.0);
        assert_eq!(deadzone(-0.06, 0.06), 0.0);
        assert!(close(deadzone(1.0, 0.06), 1.0));
        assert!(close(deadzone(-1.0, 0.06), -1.0));
        assert!(close(deadzone(0.53, 0.06), 0.5));
        assert!(close(deadzone(0.5, 0.0), 0.5));
    }

    #[test]
    fn hosas_drive_preserve_thrust_matches_bridge() {
        // Both forward: strongest wins.
        assert!(close(hosas_drive(1.0, 0.4, true), 1.0));
        assert!(close(hosas_drive(0.3, 0.8, true), 0.8));
        // One back scrubs the other's thrust proportionally.
        assert!(close(hosas_drive(0.0, 1.0, true), 1.0));
        assert!(close(hosas_drive(-0.25, 1.0, true), 0.75));
        assert!(close(hosas_drive(-0.5, 1.0, true), 0.5));
        assert!(close(hosas_drive(-1.0, 1.0, true), 0.0));
        assert!(close(hosas_drive(1.0, -0.25, true), 0.75));
        // Both back: averaged brake/reverse.
        assert!(close(hosas_drive(-1.0, -1.0, true), -1.0));
        assert!(close(hosas_drive(-1.0, 0.0, true), -0.5));
        assert!(close(hosas_drive(-0.4, -0.2, true), -0.3));
        assert!(close(hosas_drive(0.0, 0.0, true), 0.0));
    }

    #[test]
    fn hosas_drive_average_mode() {
        assert!(close(hosas_drive(1.0, 0.0, false), 0.5));
        assert!(close(hosas_drive(-1.0, -1.0, false), -1.0));
    }

    #[test]
    fn hat_cardinals_exclude_diagonals() {
        let tol = hat::DEFAULT_TOLERANCE;
        assert!(hat_matches(Some(hat::UP), hat::UP, tol));
        assert!(hat_matches(Some(35900), hat::UP, tol));
        assert!(!hat_matches(Some(hat::UP_RIGHT), hat::UP, tol));
        assert!(!hat_matches(Some(hat::UP_RIGHT), hat::RIGHT, tol));
        assert!(hat_matches(Some(hat::LEFT), hat::LEFT, tol));
        assert!(!hat_matches(None, hat::UP, tol));
    }

    #[test]
    fn report_conversions() {
        assert_eq!(to_trigger(1.0), 255);
        assert_eq!(to_trigger(-0.5), 0);
        assert_eq!(to_trigger(0.5), 128);
        assert_eq!(to_stick(1.0), 32767);
        assert_eq!(to_stick(-1.0), -32768);
        assert_eq!(to_stick(0.0), 0);
        assert!(close(normalize_di_axis(-32768), -1.0));
        assert!(close(normalize_di_axis(32767), 1.0));
    }

    #[test]
    fn linear_curve_is_identity() {
        for v in [-1.0, -0.4, 0.0, 0.25, 0.9, 1.0] {
            assert!(close(curve(v, 1.0, 0.5, 1.0), v));
        }
        assert!(close(curve(0.5, 1.0, 0.5, 0.5), 1.0));
        assert!(curve(0.5, 2.0, 0.0, 1.0) < 0.5);
    }
}
