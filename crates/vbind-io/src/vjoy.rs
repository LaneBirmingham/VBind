//! Virtual joysticks through the vJoy driver.
//!
//! vJoy is optional, so `vJoyInterface.dll` is loaded at run time. When it
//! is missing, [`Vjoy::load`] explains how to install it.

use libloading::{Library, Symbol};
use std::ffi::c_void;
use std::sync::Arc;
use vbind_core::output::VjoyReport;

pub const VJOY_DOWNLOAD: &str = "https://github.com/BrunnerInnovation/vJoy/releases";

/// `JOYSTICK_POSITION_V2` from vJoy's public.h.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JoystickPosition {
    device: u8,
    throttle: i32,
    rudder: i32,
    aileron: i32,
    axis_x: i32,
    axis_y: i32,
    axis_z: i32,
    axis_x_rot: i32,
    axis_y_rot: i32,
    axis_z_rot: i32,
    slider: i32,
    dial: i32,
    wheel: i32,
    axis_vx: i32,
    axis_vy: i32,
    axis_vz: i32,
    axis_vbrx: i32,
    axis_vbry: i32,
    axis_vbrz: i32,
    buttons: i32,
    hats: u32,
    hats_ex1: u32,
    hats_ex2: u32,
    hats_ex3: u32,
    buttons_ex1: i32,
    buttons_ex2: i32,
    buttons_ex3: i32,
}

type FnBool = unsafe extern "system" fn() -> i32;
type FnStatus = unsafe extern "system" fn(u32) -> i32;
type FnId = unsafe extern "system" fn(u32) -> i32;
type FnRelinquish = unsafe extern "system" fn(u32);
type FnUpdate = unsafe extern "system" fn(u32, *mut c_void) -> i32;
type FnAxisExist = unsafe extern "system" fn(u32, u32) -> i32;

const VJD_STAT_OWN: i32 = 0;
const VJD_STAT_FREE: i32 = 1;
const VJD_STAT_BUSY: i32 = 2;
const VJD_STAT_MISS: i32 = 3;
/// HID usages for X, Y, Z, Rx, Ry, Rz, Slider, Dial.
const AXIS_USAGES: [u32; 8] = [0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37];

/// The loaded vJoy interface library.
#[derive(Clone)]
pub struct Vjoy {
    lib: Arc<Library>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct VjoyDeviceInfo {
    pub id: u32,
    pub status: &'static str,
    pub buttons: i32,
    pub continuous_hats: i32,
    pub discrete_hats: i32,
    pub axes: Vec<&'static str>,
}

impl Vjoy {
    pub fn load() -> Result<Vjoy, String> {
        let mut candidates: Vec<String> = Vec::new();
        for var in ["ProgramFiles", "ProgramW6432"] {
            if let Ok(pf) = std::env::var(var) {
                candidates.push(format!("{pf}\\vJoy\\x64\\vJoyInterface.dll"));
            }
        }
        candidates.push("vJoyInterface.dll".into());
        for path in &candidates {
            if let Ok(lib) = unsafe { Library::new(path) } {
                let v = Vjoy { lib: Arc::new(lib) };
                if !v.enabled() {
                    return Err("vJoy is installed but disabled. Enable it in 'Configure vJoy'.".into());
                }
                return Ok(v);
            }
        }
        Err(format!("vJoy is not installed. Install the signed vJoy driver from {VJOY_DOWNLOAD}, then configure device 1 in 'Configure vJoy'."))
    }

    fn sym<T>(&self, name: &[u8]) -> Result<Symbol<'_, T>, String> {
        unsafe { self.lib.get(name) }.map_err(|e| format!("vJoyInterface.dll is missing a function: {e}"))
    }

    fn enabled(&self) -> bool {
        self.sym::<FnBool>(b"vJoyEnabled\0").map(|f| unsafe { f() } != 0).unwrap_or(false)
    }

    /// Describe vJoy devices 1-16 that are configured.
    pub fn devices(&self) -> Vec<VjoyDeviceInfo> {
        let (Ok(status), Ok(buttons), Ok(cont), Ok(disc), Ok(axis)) = (
            self.sym::<FnStatus>(b"GetVJDStatus\0"),
            self.sym::<FnId>(b"GetVJDButtonNumber\0"),
            self.sym::<FnId>(b"GetVJDContPovNumber\0"),
            self.sym::<FnId>(b"GetVJDDiscPovNumber\0"),
            self.sym::<FnAxisExist>(b"GetVJDAxisExist\0"),
        ) else {
            return Vec::new();
        };
        let names = vbind_core::state::AXIS_NAMES;
        (1..=16u32)
            .filter_map(|id| {
                let st = unsafe { status(id) };
                if st == VJD_STAT_MISS {
                    return None;
                }
                Some(VjoyDeviceInfo {
                    id,
                    status: match st {
                        VJD_STAT_OWN => "owned by VBind",
                        VJD_STAT_FREE => "free",
                        VJD_STAT_BUSY => "in use by another program",
                        _ => "unknown",
                    },
                    buttons: unsafe { buttons(id) },
                    continuous_hats: unsafe { cont(id) },
                    discrete_hats: unsafe { disc(id) },
                    axes: AXIS_USAGES
                        .iter()
                        .enumerate()
                        .filter(|(_, u)| unsafe { axis(id, **u) } != 0)
                        .map(|(i, _)| names[i])
                        .collect(),
                })
            })
            .collect()
    }

    /// Take ownership of vJoy device `id` (1-16).
    pub fn acquire(&self, id: u32) -> Result<VjoyStick, String> {
        let status = self.sym::<FnStatus>(b"GetVJDStatus\0")?;
        let acquire = self.sym::<FnStatus>(b"AcquireVJD\0")?;
        match unsafe { status(id) } {
            VJD_STAT_OWN | VJD_STAT_FREE => {}
            VJD_STAT_BUSY => return Err(format!("vJoy device {id} is in use by another program")),
            VJD_STAT_MISS => return Err(format!("vJoy device {id} is not configured. Add it in 'Configure vJoy'.")),
            _ => return Err(format!("vJoy device {id} is in an unknown state")),
        }
        if unsafe { acquire(id) } == 0 {
            return Err(format!("could not acquire vJoy device {id}"));
        }
        let cont = self.sym::<FnId>(b"GetVJDContPovNumber\0").map(|f| unsafe { f(id) }).unwrap_or(0);
        let mut stick = VjoyStick { vjoy: self.clone(), id, continuous_hats: cont > 0, last: None };
        stick.update(&VjoyReport::default())?;
        Ok(stick)
    }
}

/// An acquired vJoy device. Released when dropped.
pub struct VjoyStick {
    vjoy: Vjoy,
    id: u32,
    continuous_hats: bool,
    last: Option<VjoyReport>,
}

fn axis_value(v: f64) -> i32 {
    // vJoy axes run 1..=0x8000 with the center at 0x4000.
    let v = vbind_core::math::clamp(v, -1.0, 1.0);
    (1.0 + (v + 1.0) / 2.0 * 32767.0).round() as i32
}

impl VjoyStick {
    pub fn update(&mut self, r: &VjoyReport) -> Result<(), String> {
        if self.last.as_ref() == Some(r) {
            return Ok(());
        }
        let update = self.vjoy.sym::<FnUpdate>(b"UpdateVJD\0")?;
        let mut p = JoystickPosition { device: self.id as u8, ..Default::default() };
        let a: Vec<i32> = r.axes.iter().map(|v| axis_value(*v)).collect();
        (p.axis_x, p.axis_y, p.axis_z) = (a[0], a[1], a[2]);
        (p.axis_x_rot, p.axis_y_rot, p.axis_z_rot) = (a[3], a[4], a[5]);
        (p.slider, p.dial) = (a[6], a[7]);
        let mut words = [0u32; 4];
        for (i, pressed) in r.buttons.iter().enumerate() {
            if *pressed {
                words[i / 32] |= 1 << (i % 32);
            }
        }
        p.buttons = words[0] as i32;
        p.buttons_ex1 = words[1] as i32;
        p.buttons_ex2 = words[2] as i32;
        p.buttons_ex3 = words[3] as i32;
        if self.continuous_hats {
            let h: Vec<u32> = r.hats.iter().map(|h| h.unwrap_or(u32::MAX)).collect();
            (p.hats, p.hats_ex1, p.hats_ex2, p.hats_ex3) = (h[0], h[1], h[2], h[3]);
        } else {
            // Discrete hats: one nibble each, 0 up, 1 right, 2 down, 3 left, F centered.
            let mut nibbles = 0u32;
            for (i, h) in r.hats.iter().enumerate() {
                let n = match h {
                    Some(a) => ((a + 4500) % 36000) / 9000,
                    None => 0xF,
                };
                nibbles |= n << (i * 4);
            }
            p.hats = nibbles;
        }
        if unsafe { update(self.id, &mut p as *mut _ as *mut c_void) } == 0 {
            return Err(format!("vJoy device {} update failed", self.id));
        }
        self.last = Some(r.clone());
        Ok(())
    }

    pub fn id(&self) -> u32 {
        self.id
    }
}

impl Drop for VjoyStick {
    fn drop(&mut self) {
        let _ = self.update(&VjoyReport::default());
        if let Ok(release) = self.vjoy.sym::<FnRelinquish>(b"RelinquishVJD\0") {
            unsafe { release(self.id) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_range() {
        assert_eq!(axis_value(-1.0), 1);
        assert_eq!(axis_value(1.0), 32768);
        assert_eq!(axis_value(0.0), 16385);
        assert_eq!(std::mem::size_of::<JoystickPosition>(), 108);
    }
}
