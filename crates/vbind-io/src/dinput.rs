//! Physical device input through DirectInput 8.
//!
//! DirectInput sees every joystick, wheel, throttle and pad with all of its
//! axes (8), hats (4) and buttons (128). The objects it creates must stay on
//! the thread that created them, so [`DirectInput`] is `!Send`; the engine
//! owns it on its polling thread.

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;
use vbind_core::math::normalize_di_axis;
use vbind_core::state::{DeviceCapabilities, DeviceInfo, DeviceState, Guid, MAX_AXES, MAX_BUTTONS, MAX_HATS};
use windows::Win32::Devices::HumanInterfaceDevice::{
    DIDATAFORMAT, DIDEVCAPS, DIDEVICEINSTANCEW, DIDEVICEOBJECTINSTANCEW, DIOBJECTDATAFORMAT, DIPROPHEADER, DIPROPRANGE,
    DirectInput8Create, IDirectInput8W, IDirectInputDevice8W,
};
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPED};
use windows::core::{BOOL, GUID, w};

const DIRECTINPUT_VERSION: u32 = 0x0800;
const IID_IDIRECTINPUT8W: GUID = GUID::from_u128(0xBF798031_483A_4DA2_AA99_5D64ED369700);
const DI8DEVCLASS_GAMECTRL: u32 = 4;
const DIEDFL_ATTACHEDONLY: u32 = 1;
const DISCL_NONEXCLUSIVE: u32 = 0x2;
const DISCL_BACKGROUND: u32 = 0x8;
const DIDF_ABSAXIS: u32 = 1;
const DIDFT_AXIS: u32 = 0x3;
const DIDFT_BUTTON: u32 = 0xC;
const DIDFT_POV: u32 = 0x10;
const DIDFT_ANYINSTANCE: u32 = 0x00FF_FF00;
const DIDFT_OPTIONAL: u32 = 0x8000_0000;
const DIPH_BYOFFSET: u32 = 1;
/// `MAKEDIPROP(4)`: DirectInput property ids are small integers passed as
/// GUID pointers, not GUID values.
const DIPROP_RANGE_PTR: *const GUID = 4 as *const GUID;
const DIENUM_CONTINUE: BOOL = BOOL(1);

static GUID_X_AXIS: GUID = GUID::from_u128(0xA36D02E0_C9F3_11CF_BFC7_444553540000);
static GUID_Y_AXIS: GUID = GUID::from_u128(0xA36D02E1_C9F3_11CF_BFC7_444553540000);
static GUID_Z_AXIS: GUID = GUID::from_u128(0xA36D02E2_C9F3_11CF_BFC7_444553540000);
static GUID_RX_AXIS: GUID = GUID::from_u128(0xA36D02F4_C9F3_11CF_BFC7_444553540000);
static GUID_RY_AXIS: GUID = GUID::from_u128(0xA36D02F5_C9F3_11CF_BFC7_444553540000);
static GUID_RZ_AXIS: GUID = GUID::from_u128(0xA36D02E3_C9F3_11CF_BFC7_444553540000);
static GUID_SLIDER: GUID = GUID::from_u128(0xA36D02E4_C9F3_11CF_BFC7_444553540000);
static GUID_POV: GUID = GUID::from_u128(0xA36D02F2_C9F3_11CF_BFC7_444553540000);

/// Our device-state layout: same field order as `DIJOYSTATE2`'s position block.
#[repr(C)]
struct RawState {
    axes: [i32; MAX_AXES],
    povs: [u32; MAX_HATS],
    buttons: [u8; MAX_BUTTONS],
}

const AXIS_OFFSETS: [u32; MAX_AXES] = [0, 4, 8, 12, 16, 20, 24, 28];

fn build_format() -> (Box<[DIOBJECTDATAFORMAT]>, DIDATAFORMAT) {
    let axis_guids: [&'static GUID; MAX_AXES] = [
        &GUID_X_AXIS,
        &GUID_Y_AXIS,
        &GUID_Z_AXIS,
        &GUID_RX_AXIS,
        &GUID_RY_AXIS,
        &GUID_RZ_AXIS,
        &GUID_SLIDER,
        &GUID_SLIDER,
    ];
    let mut objs = Vec::with_capacity(MAX_AXES + MAX_HATS + MAX_BUTTONS);
    for (g, ofs) in axis_guids.iter().zip(AXIS_OFFSETS) {
        objs.push(DIOBJECTDATAFORMAT {
            pguid: *g as *const GUID,
            dwOfs: ofs,
            dwType: DIDFT_AXIS | DIDFT_ANYINSTANCE | DIDFT_OPTIONAL,
            dwFlags: 0,
        });
    }
    for i in 0..MAX_HATS as u32 {
        objs.push(DIOBJECTDATAFORMAT {
            pguid: &GUID_POV,
            dwOfs: 32 + i * 4,
            dwType: DIDFT_POV | DIDFT_ANYINSTANCE | DIDFT_OPTIONAL,
            dwFlags: 0,
        });
    }
    for i in 0..MAX_BUTTONS as u32 {
        objs.push(DIOBJECTDATAFORMAT {
            pguid: std::ptr::null(),
            dwOfs: 48 + i,
            dwType: DIDFT_BUTTON | DIDFT_ANYINSTANCE | DIDFT_OPTIONAL,
            dwFlags: 0,
        });
    }
    let mut objs = objs.into_boxed_slice();
    let fmt = DIDATAFORMAT {
        dwSize: size_of::<DIDATAFORMAT>() as u32,
        dwObjSize: size_of::<DIOBJECTDATAFORMAT>() as u32,
        dwFlags: DIDF_ABSAXIS,
        dwDataSize: size_of::<RawState>() as u32,
        dwNumObjs: objs.len() as u32,
        rgodf: objs.as_mut_ptr(),
    };
    (objs, fmt)
}

fn guid_to_u128(g: &GUID) -> u128 {
    g.to_u128()
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end]).trim().to_string()
}

/// VID/PID from a DirectInput product GUID (`MAKELONG(vid, pid)` in Data1
/// with "PIDVID" in Data4 for HID devices).
fn vid_pid(product: &GUID) -> (Option<u16>, Option<u16>) {
    if &product.data4[2..8] == b"PIDVID" {
        (Some((product.data1 & 0xFFFF) as u16), Some((product.data1 >> 16) as u16))
    } else {
        (None, None)
    }
}

struct Device {
    info: DeviceInfo,
    dev: IDirectInputDevice8W,
    /// Whether the last read succeeded.
    connected: bool,
}

/// All attached game controllers, read through DirectInput.
pub struct DirectInput {
    di: IDirectInput8W,
    hwnd: HWND,
    _objs: Box<[DIOBJECTDATAFORMAT]>,
    format: DIDATAFORMAT,
    devices: Vec<Device>,
}

impl DirectInput {
    pub fn new() -> windows::core::Result<DirectInput> {
        unsafe {
            let hinst = HINSTANCE(GetModuleHandleW(None)?.0);
            let di = create_di()?;
            // Background, non-exclusive access needs a top-level window to
            // attach to; it is never shown.
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("VBind input"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(hinst),
                None,
            )?;
            let (objs, format) = build_format();
            let mut me = DirectInput { di, hwnd, _objs: objs, format, devices: Vec::new() };
            me.refresh();
            Ok(me)
        }
    }

    /// Connected devices.
    pub fn devices(&self) -> impl Iterator<Item = &DeviceInfo> {
        self.devices.iter().filter(|d| d.connected).map(|d| &d.info)
    }

    /// Enumerate attached game controllers right now and open or drop
    /// devices to match. This can take 100 ms or more, so the engine uses a
    /// [`Scanner`] instead and only calls [`sync`](Self::sync).
    pub fn refresh(&mut self) -> bool {
        match enumerate(&self.di) {
            Some(found) => self.sync(&found),
            None => false,
        }
    }

    /// Open devices that appeared in `found` and drop ones that are gone.
    /// Returns true when the set of connected devices changed.
    pub fn sync(&mut self, found: &[AttachedDevice]) -> bool {
        let before: Vec<(Guid, bool)> = self.devices.iter().map(|d| (d.info.instance, d.connected)).collect();
        let present: Vec<Guid> = found.iter().map(AttachedDevice::instance).collect();
        self.devices.retain(|d| present.contains(&d.info.instance));
        for inst in found {
            let id = inst.instance();
            if let Some(d) = self.devices.iter_mut().find(|d| d.info.instance == id) {
                if !d.connected {
                    unsafe {
                        let _ = d.dev.Acquire();
                    }
                    d.connected = true;
                }
                continue;
            }
            if let Some(d) = self.open(&inst.0) {
                self.devices.push(d);
            }
        }
        let after: Vec<(Guid, bool)> = self.devices.iter().map(|d| (d.info.instance, d.connected)).collect();
        before != after
    }

    fn open(&mut self, inst: &DIDEVICEINSTANCEW) -> Option<Device> {
        unsafe {
            let mut dev: Option<IDirectInputDevice8W> = None;
            self.di.CreateDevice(&inst.guidInstance, &mut dev, None).ok()?;
            let dev = dev?;
            dev.SetDataFormat(&mut self.format).ok()?;
            for ofs in AXIS_OFFSETS {
                let mut range = DIPROPRANGE {
                    diph: DIPROPHEADER {
                        dwSize: size_of::<DIPROPRANGE>() as u32,
                        dwHeaderSize: size_of::<DIPROPHEADER>() as u32,
                        dwObj: ofs,
                        dwHow: DIPH_BYOFFSET,
                    },
                    lMin: -32768,
                    lMax: 32767,
                };
                let _ = dev.SetProperty(DIPROP_RANGE_PTR, &mut range.diph);
            }
            dev.SetCooperativeLevel(self.hwnd, DISCL_BACKGROUND | DISCL_NONEXCLUSIVE).ok()?;
            let _ = dev.Acquire();

            let mut caps = DIDEVCAPS { dwSize: size_of::<DIDEVCAPS>() as u32, ..Default::default() };
            let _ = dev.GetCapabilities(&mut caps);
            let mut axes = [false; MAX_AXES];
            for (i, ofs) in AXIS_OFFSETS.iter().enumerate() {
                let mut obj = DIDEVICEOBJECTINSTANCEW {
                    dwSize: size_of::<DIDEVICEOBJECTINSTANCEW>() as u32,
                    ..Default::default()
                };
                axes[i] = dev.GetObjectInfo(&mut obj, *ofs, DIPH_BYOFFSET).is_ok();
            }
            let (vid, pid) = vid_pid(&inst.guidProduct);
            let mut name = wide_to_string(&inst.tszProductName);
            if name.is_empty() {
                name = wide_to_string(&inst.tszInstanceName);
            }
            Some(Device {
                info: DeviceInfo {
                    instance: Guid(guid_to_u128(&inst.guidInstance)),
                    vid,
                    pid,
                    name,
                    caps: DeviceCapabilities {
                        axes,
                        hat_count: caps.dwPOVs.min(MAX_HATS as u32) as u8,
                        button_count: caps.dwButtons.min(MAX_BUTTONS as u32) as u8,
                    },
                },
                dev,
                connected: true,
            })
        }
    }

    /// Read every device. Devices that fail to read are reported as `None`
    /// and dropped from [`devices`](Self::devices) until they come back.
    pub fn poll(&mut self, mut each: impl FnMut(&DeviceInfo, Option<DeviceState>)) -> bool {
        let mut lost = false;
        for d in self.devices.iter_mut() {
            let state = read(&d.dev, &d.info.caps);
            if state.is_none() && d.connected {
                lost = true;
            }
            d.connected = state.is_some();
            each(&d.info, state);
        }
        lost
    }
}

fn read(dev: &IDirectInputDevice8W, caps: &DeviceCapabilities) -> Option<DeviceState> {
    let mut raw = RawState { axes: [0; MAX_AXES], povs: [u32::MAX; MAX_HATS], buttons: [0; MAX_BUTTONS] };
    let size = size_of::<RawState>() as u32;
    let ptr = &mut raw as *mut RawState as *mut c_void;
    unsafe {
        let _ = dev.Poll();
        if dev.GetDeviceState(size, ptr).is_err() {
            let _ = dev.Acquire();
            let _ = dev.Poll();
            dev.GetDeviceState(size, ptr).ok()?;
        }
    }
    let mut s = DeviceState::default();
    for (o, r) in s.axes.iter_mut().zip(raw.axes) {
        *o = normalize_di_axis(r);
    }
    // Absent hats and buttons are not guaranteed to read as released.
    for (o, r) in s.hats.iter_mut().zip(raw.povs).take(caps.hat_count as usize) {
        *o = if r & 0xFFFF == 0xFFFF { None } else { Some(r % 36000) };
    }
    for (o, r) in s.buttons.iter_mut().zip(raw.buttons).take(caps.button_count as usize) {
        *o = r & 0x80 != 0;
    }
    for (o, present) in s.axes.iter_mut().zip(caps.axes) {
        if !present {
            *o = 0.0;
        }
    }
    Some(s)
}

impl Drop for DirectInput {
    fn drop(&mut self) {
        unsafe {
            for d in &self.devices {
                let _ = d.dev.Unacquire();
            }
            self.devices.clear();
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// One attached device as reported by enumeration. Plain data, so it can be
/// handed from the scanner thread to the engine thread.
#[derive(Clone, Copy)]
pub struct AttachedDevice(DIDEVICEINSTANCEW);

// SAFETY: DIDEVICEINSTANCEW holds only GUIDs, integers and fixed arrays.
unsafe impl Send for AttachedDevice {}

impl AttachedDevice {
    pub fn instance(&self) -> Guid {
        Guid(guid_to_u128(&self.0.guidInstance))
    }
}

fn create_di() -> windows::core::Result<IDirectInput8W> {
    unsafe {
        let module = GetModuleHandleW(None)?;
        let mut raw: *mut c_void = std::ptr::null_mut();
        DirectInput8Create(HINSTANCE(module.0), DIRECTINPUT_VERSION, &IID_IDIRECTINPUT8W, &mut raw, None)?;
        Ok(<IDirectInput8W as windows::core::Interface>::from_raw(raw))
    }
}

fn enumerate(di: &IDirectInput8W) -> Option<Vec<AttachedDevice>> {
    let mut found: Vec<AttachedDevice> = Vec::new();
    unsafe extern "system" fn on_device(inst: *mut DIDEVICEINSTANCEW, ctx: *mut c_void) -> BOOL {
        unsafe {
            let list = &mut *(ctx as *mut Vec<AttachedDevice>);
            list.push(AttachedDevice(*inst));
        }
        DIENUM_CONTINUE
    }
    let ok = unsafe { di.EnumDevices(DI8DEVCLASS_GAMECTRL, Some(on_device), &mut found as *mut _ as *mut c_void, DIEDFL_ATTACHEDONLY) };
    ok.ok().map(|_| found)
}

/// Enumerates devices on its own thread (enumeration is slow) and sends the
/// list whenever it changes, or when woken.
pub struct Scanner {
    pub updates: Receiver<Vec<AttachedDevice>>,
    wake: Sender<()>,
}

impl Scanner {
    pub fn start(interval: Duration) -> Scanner {
        let (tx, updates) = mpsc::channel();
        let (wake, wake_rx) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("vbind-scan".into())
            .spawn(move || {
                let Ok(di) = create_di() else { return };
                let mut last: Option<Vec<Guid>> = None;
                loop {
                    if let Some(found) = enumerate(&di) {
                        let ids: Vec<Guid> = found.iter().map(AttachedDevice::instance).collect();
                        if last.as_ref() != Some(&ids) {
                            if tx.send(found).is_err() {
                                return;
                            }
                            last = Some(ids);
                        }
                    }
                    match wake_rx.recv_timeout(interval) {
                        Ok(()) => {
                            // Coalesce a burst of wake requests into one scan.
                            while wake_rx.try_recv().is_ok() {}
                            last = None;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .expect("spawn scanner thread");
        Scanner { updates, wake }
    }

    /// Scan again now (and report the list even if unchanged).
    pub fn wake(&self) {
        let _ = self.wake.send(());
    }
}
