//! The engine thread: polls physical devices, runs the active profile, feeds
//! virtual devices, and handles input capture and simulation.
//!
//! DirectInput objects are bound to the thread that made them, so everything
//! device-related happens here. Other threads talk to it with [`Command`]s and
//! read the latest [`Snapshot`].

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use vbind_core::expr::HatDir;
use vbind_core::math;
use vbind_core::output::{OutputKind, Report};
use vbind_core::profile::{CompiledProfile, Runtime};
use vbind_core::state::{DeviceInfo, DeviceState, Guid};
use vbind_io::dinput::{DirectInput, Scanner};
use vbind_io::vigem::{Vigem, XboxPad};
use vbind_io::vjoy::{Vjoy, VjoyDeviceInfo, VjoyStick};

const IDLE_HZ: u32 = 250;
const PUBLISH_EVERY: Duration = Duration::from_millis(16);
const SCAN_EVERY: Duration = Duration::from_secs(1);
const XBOX360_VID_PID: (u16, u16) = (0x045E, 0x028E);

pub struct ActivateSpec {
    pub compiled: Arc<CompiledProfile>,
    /// Device for each role, by role index.
    pub roles: Vec<Option<Guid>>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Captured {
    pub device: Guid,
    pub name: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    /// Control id usable in expressions after the role: `b1`, `hat1.up`, `y`.
    pub control: String,
    pub kind: &'static str,
    /// Axis position when captured (sign shows direction), 1 for buttons and hats.
    pub value: f64,
}

#[derive(Clone, Debug)]
pub struct CaptureRequest {
    pub prompt: String,
    pub buttons: bool,
    pub hats: bool,
    pub axes: bool,
    pub devices: Option<Vec<Guid>>,
    pub timeout: Duration,
    pub axis_threshold: f64,
}

/// `Ok(None)` means nothing was pressed before the timeout.
pub type CaptureResult = Result<Option<Captured>, String>;

#[derive(Clone, Debug, Default)]
pub struct Simulation {
    pub device: Guid,
    pub axes: Vec<(usize, f64)>,
    pub buttons: Vec<(usize, bool)>,
    pub hats: Vec<(usize, Option<u32>)>,
    pub until: Option<Instant>,
}

pub enum Command {
    Activate(ActivateSpec, oneshot::Sender<Result<(), String>>),
    UpdateRoles(Vec<Option<Guid>>),
    Deactivate(oneshot::Sender<()>),
    Capture(CaptureRequest, oneshot::Sender<CaptureResult>),
    CancelCapture,
    Simulate(Simulation),
    ClearSimulation(Option<Guid>),
    RefreshOutputs,
    Shutdown,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct OutputSnapshot {
    pub name: String,
    pub kind: OutputKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xinput_slot: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vjoy_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip)]
    pub report: Option<Report>,
}

#[derive(Clone, Debug)]
pub struct ActiveSnapshot {
    pub compiled: Arc<CompiledProfile>,
    pub roles: Vec<Option<Guid>>,
    pub vars: Vec<f64>,
    pub values: Vec<f64>,
    pub outputs: Vec<OutputSnapshot>,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub seq: u64,
    pub devices: Vec<DeviceInfo>,
    pub states: HashMap<Guid, DeviceState>,
    pub active: Option<ActiveSnapshot>,
    pub vigem_error: Option<String>,
    pub vjoy_error: Option<String>,
    pub vjoy_devices: Vec<VjoyDeviceInfo>,
    pub input_error: Option<String>,
    pub measured_hz: f64,
    pub capture: Option<(String, Instant)>,
    pub simulated: Vec<Guid>,
}

pub struct Shared {
    pub snapshot: RwLock<Arc<Snapshot>>,
    /// Bumped whenever the set of connected devices changes.
    pub devices_rev: AtomicU64,
}

#[derive(Clone)]
pub struct EngineHandle {
    tx: Sender<Command>,
    pub shared: Arc<Shared>,
}

impl EngineHandle {
    pub fn start() -> EngineHandle {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared { snapshot: RwLock::new(Arc::new(Snapshot::default())), devices_rev: AtomicU64::new(0) });
        let s2 = shared.clone();
        std::thread::Builder::new()
            .name("vbind-engine".into())
            .spawn(move || Engine::new(rx, s2).run())
            .expect("spawn engine thread");
        EngineHandle { tx, shared }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.shared.snapshot.read().unwrap().clone()
    }

    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }

    pub async fn activate(&self, spec: ActivateSpec) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.send(Command::Activate(spec, tx));
        rx.await.unwrap_or_else(|_| Err("engine stopped".into()))
    }

    pub async fn deactivate(&self) {
        let (tx, rx) = oneshot::channel();
        self.send(Command::Deactivate(tx));
        let _ = rx.await;
    }

    pub async fn capture(&self, req: CaptureRequest) -> CaptureResult {
        let (tx, rx) = oneshot::channel();
        self.send(Command::Capture(req, tx));
        rx.await.unwrap_or_else(|_| Err("capture cancelled".into()))
    }
}

enum OutputDev {
    Xbox { pad: XboxPad, slot: Option<u32> },
    Vjoy(VjoyStick),
}

struct OutputSlot {
    kind: OutputKind,
    vjoy_id: u8,
    dev: OutputDev,
    error: Option<String>,
}

struct Active {
    compiled: Arc<CompiledProfile>,
    runtime: Runtime,
    roles: Vec<Option<Guid>>,
}

struct CaptureJob {
    req: CaptureRequest,
    baseline: HashMap<Guid, DeviceState>,
    started: Instant,
    reply: oneshot::Sender<CaptureResult>,
}

struct Engine {
    rx: Receiver<Command>,
    shared: Arc<Shared>,
    di: Option<DirectInput>,
    scanner: Option<Scanner>,
    last_wake: Instant,
    di_retry: Instant,
    input_error: Option<String>,
    vigem: Option<Vigem>,
    vigem_error: Option<String>,
    vjoy: Option<Vjoy>,
    vjoy_error: Option<String>,
    devices: Vec<DeviceInfo>,
    states: HashMap<Guid, DeviceState>,
    excluded: HashSet<Guid>,
    /// While set, newly appearing Xbox 360 pads are VBind's own.
    watch_virtual: Option<(Instant, HashSet<Guid>)>,
    outputs: Vec<OutputSlot>,
    active: Option<Active>,
    capture: Option<CaptureJob>,
    sims: Vec<Simulation>,
    seq: u64,
    ticks: u32,
    tick_window: Instant,
    measured_hz: f64,
}

impl Engine {
    fn new(rx: Receiver<Command>, shared: Arc<Shared>) -> Engine {
        let now = Instant::now();
        let mut e = Engine {
            rx,
            shared,
            di: None,
            scanner: None,
            last_wake: now,
            di_retry: now,
            input_error: None,
            vigem: None,
            vigem_error: None,
            vjoy: None,
            vjoy_error: None,
            devices: Vec::new(),
            states: HashMap::new(),
            excluded: HashSet::new(),
            watch_virtual: None,
            outputs: Vec::new(),
            active: None,
            capture: None,
            sims: Vec::new(),
            seq: 0,
            ticks: 0,
            tick_window: now,
            measured_hz: 0.0,
        };
        e.check_drivers();
        e
    }

    fn check_drivers(&mut self) {
        if self.vigem.is_none() {
            match Vigem::connect() {
                Ok(v) => {
                    self.vigem = Some(v);
                    self.vigem_error = None;
                }
                Err(e) => self.vigem_error = Some(e),
            }
        }
        if self.vjoy.is_none() {
            match Vjoy::load() {
                Ok(v) => {
                    self.vjoy = Some(v);
                    self.vjoy_error = None;
                }
                Err(e) => self.vjoy_error = Some(e),
            }
        }
    }

    fn run(mut self) {
        let mut next = Instant::now();
        let mut last_publish = Instant::now() - PUBLISH_EVERY;
        let trace = std::env::var_os("VBIND_TRACE").is_some();
        let mut spent = [Duration::ZERO; 3];
        let mut trace_at = Instant::now();
        loop {
            loop {
                match self.rx.try_recv() {
                    Ok(Command::Shutdown) => {
                        self.deactivate();
                        return;
                    }
                    Ok(cmd) => self.handle(cmd),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.deactivate();
                        return;
                    }
                }
            }
            let t0 = Instant::now();
            self.tick();
            let t1 = Instant::now();
            if last_publish.elapsed() >= PUBLISH_EVERY {
                self.publish();
                last_publish = Instant::now();
            }
            let t2 = Instant::now();
            if trace {
                spent[0] += t1 - t0;
                spent[1] += t2 - t1;
                if trace_at.elapsed() >= Duration::from_secs(1) {
                    eprintln!("[engine] {:.0} Hz  tick {:.1} ms/s  publish {:.1} ms/s  sleep {:.1} ms/s", self.measured_hz, spent[0].as_secs_f64() * 1e3, spent[1].as_secs_f64() * 1e3, spent[2].as_secs_f64() * 1e3);
                    spent = [Duration::ZERO; 3];
                    trace_at = Instant::now();
                }
            }
            let hz = self.active.as_ref().map(|a| a.compiled.polling_hz).unwrap_or(IDLE_HZ).clamp(50, 1000);
            next += Duration::from_secs_f64(1.0 / hz as f64);
            let now = Instant::now();
            if next <= now {
                next = now;
            } else {
                std::thread::sleep(next - now);
                spent[2] += now.elapsed();
            }
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Activate(spec, reply) => {
                let r = self.activate(spec);
                let _ = reply.send(r);
            }
            Command::UpdateRoles(roles) => {
                if let Some(a) = self.active.as_mut() {
                    a.roles = roles;
                }
            }
            Command::Deactivate(reply) => {
                self.deactivate();
                let _ = reply.send(());
            }
            Command::Capture(req, reply) => {
                if let Some(old) = self.capture.take() {
                    let _ = old.reply.send(Err("replaced by a newer capture".into()));
                }
                self.capture = Some(CaptureJob { baseline: self.states.clone(), started: Instant::now(), req, reply });
            }
            Command::CancelCapture => {
                if let Some(old) = self.capture.take() {
                    let _ = old.reply.send(Err("capture cancelled".into()));
                }
            }
            Command::Simulate(s) => {
                self.sims.push(s);
            }
            Command::ClearSimulation(dev) => match dev {
                Some(g) => self.sims.retain(|s| s.device != g),
                None => self.sims.clear(),
            },
            Command::RefreshOutputs => {
                if self.vjoy.is_none() || self.vigem.is_none() {
                    self.check_drivers();
                }
            }
            Command::Shutdown => {}
        }
    }

    fn activate(&mut self, spec: ActivateSpec) -> Result<(), String> {
        self.check_drivers();
        // Reuse matching virtual devices so a game never sees the pad unplug
        // when a profile is edited or swapped.
        let mut old: Vec<Option<OutputSlot>> = std::mem::take(&mut self.outputs).into_iter().map(Some).collect();
        let mut fresh = Vec::new();
        for o in &spec.compiled.outputs {
            let reuse = old.iter_mut().find(|s| s.as_ref().is_some_and(|s| s.kind == o.kind && (o.kind != OutputKind::Vjoy || s.vjoy_id == o.vjoy_id)));
            if let Some(slot) = reuse.and_then(Option::take) {
                fresh.push(slot);
                continue;
            }
            let made = match o.kind {
                OutputKind::Xbox360 => self.plug_xbox(),
                OutputKind::Vjoy => match &self.vjoy {
                    Some(v) => v.acquire(o.vjoy_id as u32).map(OutputDev::Vjoy),
                    None => Err(self.vjoy_error.clone().unwrap_or_else(|| "vJoy is not installed".into())),
                },
            };
            match made {
                Ok(dev) => fresh.push(OutputSlot { kind: o.kind, vjoy_id: o.vjoy_id, dev, error: None }),
                Err(e) => {
                    // Roll back: keep nothing half-made.
                    drop(fresh);
                    drop(old);
                    self.active = None;
                    return Err(format!("output '{}': {e}", o.name));
                }
            }
        }
        drop(old);
        self.outputs = fresh;
        let runtime = spec.compiled.runtime();
        self.active = Some(Active { compiled: spec.compiled, runtime, roles: spec.roles });
        Ok(())
    }

    fn plug_xbox(&mut self) -> Result<OutputDev, String> {
        let Some(bus) = &self.vigem else {
            return Err(self.vigem_error.clone().unwrap_or_else(|| "ViGEmBus is not available".into()));
        };
        let before: HashSet<Guid> = self.devices.iter().map(|d| d.instance).collect();
        let mut pad = bus.plug_xbox360()?;
        let slot = pad.user_index();
        let known = match self.watch_virtual.take() {
            Some((_, mut k)) => {
                k.extend(before);
                k
            }
            None => before,
        };
        self.watch_virtual = Some((Instant::now() + Duration::from_secs(4), known));
        Ok(OutputDev::Xbox { pad, slot })
    }

    fn deactivate(&mut self) {
        self.active = None;
        self.outputs.clear();
    }

    /// Apply the scanner's latest device list. Enumeration itself happens on
    /// the scanner thread because it can take over 100 ms.
    fn sync_devices(&mut self, initial: bool) {
        let Some(di) = self.di.as_mut() else { return };
        let watching = self.watch_virtual.as_ref().is_some_and(|(until, _)| Instant::now() < *until);
        if let Some(sc) = &self.scanner {
            if watching && self.last_wake.elapsed() >= Duration::from_millis(250) {
                sc.wake();
                self.last_wake = Instant::now();
            }
        }
        let mut latest = None;
        if let Some(sc) = &self.scanner {
            while let Ok(list) = sc.updates.try_recv() {
                latest = Some(list);
            }
        }
        let changed = match &latest {
            Some(list) => di.sync(list),
            None => false,
        };
        if !changed && !initial {
            if !watching {
                self.watch_virtual = None;
            }
            return;
        }
        let all: Vec<DeviceInfo> = di.devices().cloned().collect();
        if let Some((until, known)) = &self.watch_virtual {
            for d in &all {
                if !known.contains(&d.instance) && (d.vid, d.pid) == (Some(XBOX360_VID_PID.0), Some(XBOX360_VID_PID.1)) {
                    self.excluded.insert(d.instance);
                }
            }
            if Instant::now() >= *until {
                self.watch_virtual = None;
            }
        }
        let list: Vec<DeviceInfo> = all
            .into_iter()
            .filter(|d| !self.excluded.contains(&d.instance))
            .filter(|d| (d.vid, d.pid) != (Some(vbind_io::VJOY_VID_PID.0), Some(vbind_io::VJOY_VID_PID.1)))
            .collect();
        let ids = |v: &[DeviceInfo]| v.iter().map(|d| d.instance).collect::<Vec<_>>();
        if ids(&list) != ids(&self.devices) || initial {
            self.shared.devices_rev.fetch_add(1, Ordering::SeqCst);
        }
        self.devices = list;
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.di.is_none() && now >= self.di_retry {
            match DirectInput::new() {
                Ok(d) => {
                    self.di = Some(d);
                    self.scanner = Some(Scanner::start(SCAN_EVERY));
                    self.input_error = None;
                    self.sync_devices(true);
                }
                Err(e) => {
                    self.input_error = Some(format!("DirectInput unavailable: {e}"));
                    self.di_retry = now + Duration::from_secs(5);
                }
            }
        }
        self.sync_devices(false);

        let mut lost = false;
        if let Some(di) = self.di.as_mut() {
            let states = &mut self.states;
            let excluded = &self.excluded;
            lost = di.poll(|info, st| {
                if excluded.contains(&info.instance) {
                    return;
                }
                match st {
                    Some(s) => {
                        states.insert(info.instance, s);
                    }
                    None => {
                        states.remove(&info.instance);
                    }
                }
            });
        }
        if lost {
            // A device stopped answering: drop it from the list now and let
            // the scanner confirm whether it is gone.
            if let Some(di) = self.di.as_ref() {
                let connected: Vec<Guid> = di.devices().map(|d| d.instance).collect();
                let before = self.devices.len();
                self.devices.retain(|d| connected.contains(&d.instance));
                if self.devices.len() != before {
                    self.shared.devices_rev.fetch_add(1, Ordering::SeqCst);
                }
            }
            if let Some(sc) = &self.scanner {
                sc.wake();
            }
        }
        self.states.retain(|g, _| self.devices.iter().any(|d| d.instance == *g));

        self.sims.retain(|s| s.until.is_none_or(|u| now < u));
        for s in &self.sims {
            if let Some(st) = self.states.get_mut(&s.device) {
                for (i, v) in &s.axes {
                    st.axes[*i] = *v;
                }
                for (i, v) in &s.buttons {
                    st.buttons[*i] = *v;
                }
                for (i, v) in &s.hats {
                    st.hats[*i] = *v;
                }
            }
        }

        self.check_capture();

        if let Some(a) = self.active.as_mut() {
            let devs: Vec<Option<&DeviceState>> = a.roles.iter().map(|r| r.and_then(|g| self.states.get(&g))).collect();
            a.compiled.evaluate(&devs, &mut a.runtime);
            for (slot, report) in self.outputs.iter_mut().zip(&a.runtime.reports) {
                let r = match (&mut slot.dev, report) {
                    (OutputDev::Xbox { pad, .. }, Report::Xbox(x)) => pad.update(x),
                    (OutputDev::Vjoy(v), Report::Vjoy(j)) => v.update(j),
                    _ => Ok(()),
                };
                slot.error = r.err();
            }
        }

        self.ticks += 1;
        let elapsed = self.tick_window.elapsed();
        if elapsed >= Duration::from_secs(1) {
            self.measured_hz = self.ticks as f64 / elapsed.as_secs_f64();
            self.ticks = 0;
            self.tick_window = Instant::now();
        }
    }

    fn check_capture(&mut self) {
        let Some(job) = self.capture.as_mut() else { return };
        if job.reply.is_closed() {
            self.capture = None;
            return;
        }
        let mut found: Option<Captured> = None;
        'devices: for info in &self.devices {
            if job.req.devices.as_ref().is_some_and(|only| !only.contains(&info.instance)) {
                continue;
            }
            let Some(now) = self.states.get(&info.instance) else { continue };
            let base = job.baseline.entry(info.instance).or_insert_with(|| now.clone());
            let make = |control: String, kind: &'static str, value: f64| Captured {
                device: info.instance,
                name: info.name.clone(),
                vid: info.vid,
                pid: info.pid,
                control,
                kind,
                value,
            };
            if job.req.buttons {
                for i in 0..now.buttons.len() {
                    if now.buttons[i] && !base.buttons[i] {
                        found = Some(make(format!("b{}", i + 1), "button", 1.0));
                        break 'devices;
                    }
                    if !now.buttons[i] {
                        base.buttons[i] = false;
                    }
                }
            }
            if job.req.hats {
                for h in 0..now.hats.len() {
                    if let Some(angle) = now.hats[h] {
                        if base.hats[h].is_none() {
                            let dir = HatDir::ALL
                                .into_iter()
                                .min_by_key(|d| math::hat_angle_distance(angle, d.angle()))
                                .unwrap_or(HatDir::Up);
                            found = Some(make(format!("hat{}.{}", h + 1, dir.name()), "hat", 1.0));
                            break 'devices;
                        }
                    } else {
                        base.hats[h] = None;
                    }
                }
            }
            if job.req.axes {
                for a in 0..now.axes.len() {
                    if info.caps.axes[a] && (now.axes[a] - base.axes[a]).abs() >= job.req.axis_threshold {
                        found = Some(make(vbind_core::state::AXIS_NAMES[a].to_string(), "axis", now.axes[a] - base.axes[a]));
                        break 'devices;
                    }
                }
            }
        }
        if let Some(c) = found {
            let job = self.capture.take().unwrap();
            let _ = job.reply.send(Ok(Some(c)));
        } else if job.started.elapsed() >= job.req.timeout {
            let job = self.capture.take().unwrap();
            let _ = job.reply.send(Ok(None));
        }
    }

    fn publish(&mut self) {
        self.seq += 1;
        let active = self.active.as_ref().map(|a| ActiveSnapshot {
            compiled: a.compiled.clone(),
            roles: a.roles.clone(),
            vars: a.runtime.vars.clone(),
            values: a.runtime.values.clone(),
            outputs: a
                .compiled
                .outputs
                .iter()
                .zip(self.outputs.iter())
                .zip(&a.runtime.reports)
                .map(|((o, slot), rep)| OutputSnapshot {
                    name: o.name.clone(),
                    kind: o.kind,
                    xinput_slot: match &slot.dev {
                        OutputDev::Xbox { slot, .. } => *slot,
                        _ => None,
                    },
                    vjoy_id: match &slot.dev {
                        OutputDev::Vjoy(v) => Some(v.id()),
                        _ => None,
                    },
                    error: slot.error.clone(),
                    report: Some(rep.clone()),
                })
                .collect(),
        });
        let snap = Snapshot {
            seq: self.seq,
            devices: self.devices.clone(),
            states: self.states.clone(),
            active,
            vigem_error: self.vigem_error.clone(),
            vjoy_error: self.vjoy_error.clone(),
            vjoy_devices: self.vjoy.as_ref().map(|v| if self.seq % 60 == 1 { v.devices() } else { Vec::new() }).unwrap_or_default(),
            input_error: self.input_error.clone(),
            measured_hz: self.measured_hz,
            capture: self.capture.as_ref().map(|c| (c.req.prompt.clone(), c.started)),
            simulated: self.sims.iter().map(|s| s.device).collect(),
        };
        let mut snap = snap;
        if snap.vjoy_devices.is_empty() && self.vjoy.is_some() {
            // vJoy device listing is slow; keep the previous one between refreshes.
            snap.vjoy_devices = self.shared.snapshot.read().unwrap().vjoy_devices.clone();
        }
        *self.shared.snapshot.write().unwrap() = Arc::new(snap);
    }
}
