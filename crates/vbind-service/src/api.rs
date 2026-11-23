//! The local HTTP API on 127.0.0.1. The window, the `vbind` CLI and the MCP
//! server all use it. Every `/api` call needs the token from runtime.json.

use crate::engine::{ActivateSpec, CaptureRequest, Command, EngineHandle, Simulation, Snapshot};
use crate::import_hosas;
use crate::panel::{Panel, PanelPost};
use crate::store::Store;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use futures_util::stream::{self, Stream, StreamExt};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;
use vbind_core::expr::{self, Env, EvalState, InputControl, Resolver, Scope};
use vbind_core::graph::build_graph;
use vbind_core::layout::DeviceLayout;
use vbind_core::output::Report;
use vbind_core::profile::{Diagnostic, Profile};
use vbind_core::resolve::{RoleResolution, resolve_roles};
use vbind_core::state::{AXIS_NAMES, DeviceInfo, DeviceState, Guid};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(rust_embed::Embed)]
#[folder = "../../ui/dist"]
#[allow_missing = true]
struct UiAssets;

#[derive(Default)]
pub struct AgentSeen {
    pub client: Option<String>,
    pub at: Option<Instant>,
}

/// Why the saved profile is not running yet.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Pending {
    pub profile: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<RoleResolution>,
}

pub struct AppState {
    pub engine: EngineHandle,
    pub store: Arc<Store>,
    pub panel: Arc<Panel>,
    pub token: String,
    pub port: u16,
    pub changes: broadcast::Sender<String>,
    pub agent: Mutex<AgentSeen>,
    pub pending: Mutex<Option<Pending>>,
    pub started: Instant,
}

pub type Shared = Arc<AppState>;

// ---------------------------------------------------------------- errors

pub struct ApiError {
    status: StatusCode,
    body: Value,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> ApiError {
        ApiError { status, body: json!({ "error": message.into() }) }
    }
    fn with(mut self, key: &str, v: impl serde::Serialize) -> ApiError {
        self.body[key] = serde_json::to_value(v).unwrap_or(Value::Null);
        self
    }
    fn not_found(what: &str, id: &str) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, format!("no {what} '{id}'"))
    }
    fn bad(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, message)
    }
    fn invalid(diags: Vec<Diagnostic>) -> ApiError {
        let summary = diags.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("; ");
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("profile has problems: {summary}")).with("diagnostics", diags)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

type ApiResult<T = Json<Value>> = Result<T, ApiError>;

fn ok(v: impl serde::Serialize) -> ApiResult {
    Ok(Json(serde_json::to_value(v).unwrap_or(Value::Null)))
}

// ---------------------------------------------------------------- router

pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/guide", get(guide))
        .route("/schema/{kind}", get(schema))
        .route("/devices", get(devices))
        .route("/state", get(live_state))
        .route("/events", get(events))
        .route("/capture", post(capture).delete(cancel_capture))
        .route("/simulate", post(simulate).delete(clear_simulation))
        .route("/xinput", get(xinput))
        .route("/eval", post(eval_expr))
        .route("/profiles", get(list_profiles).post(import_profile))
        .route("/profiles/validate", post(validate_profile))
        .route("/profiles/{id}", get(get_profile).put(put_profile).delete(delete_profile))
        .route("/profiles/{id}/activate", post(activate))
        .route("/profiles/{id}/graph", get(graph))
        .route("/profiles/{id}/roles", get(roles))
        .route("/profiles/{id}/assignments", put(assign))
        .route("/profiles/{id}/map", post(edit_map))
        .route("/deactivate", post(deactivate))
        .route("/layouts", get(list_layouts).post(save_layout_post))
        .route("/layouts/{id}", get(get_layout).put(save_layout_put).delete(delete_layout))
        .route("/panel", get(get_panel).post(post_panel).delete(clear_panel))
        .route("/panel/ask", post(ask))
        .route("/panel/answer", post(answer))
        .route("/import/hosas-bridge", post(import_bridge))
        .route("/reload", post(reload))
        .layer(middleware::from_fn_with_state(state.clone(), auth));
    Router::new()
        .nest("/api", api)
        .fallback(get(ui_asset))
        .layer(middleware::from_fn_with_state(state.clone(), host_check))
        .with_state(state)
}

/// Only answer requests addressed to this machine by name or loopback IP
/// (blocks DNS-rebinding pages from talking to the API).
async fn host_check(State(st): State<Shared>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let allowed = [format!("127.0.0.1:{}", st.port), format!("localhost:{}", st.port)];
    if !allowed.iter().any(|a| a == host) {
        return (StatusCode::FORBIDDEN, "VBind only answers on 127.0.0.1").into_response();
    }
    let mut res = next.run(req).await;
    res.headers_mut().insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    res
}

async fn auth(State(st): State<Shared>, req: Request, next: Next) -> Response {
    let h = req.headers();
    let bearer = h
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    let header_token = h.get("x-vbind-token").and_then(|v| v.to_str().ok()).map(str::to_string);
    let query_token = req.uri().query().and_then(|q| {
        q.split('&').find_map(|kv| kv.strip_prefix("token=")).map(str::to_string)
    });
    let given = bearer.or(header_token).or(query_token);
    if given.as_deref() != Some(st.token.as_str()) {
        return ApiError::new(StatusCode::UNAUTHORIZED, "missing or wrong VBind token; read it from %APPDATA%\\VBind\\runtime.json").into_response();
    }
    if let Some(client) = h.get("x-vbind-client").and_then(|v| v.to_str().ok()) {
        if client != "ui" {
            let mut a = st.agent.lock().unwrap();
            a.client = Some(client.to_string());
            a.at = Some(Instant::now());
        }
    }
    next.run(req).await
}

fn changed(st: &AppState, what: &str) {
    let _ = st.changes.send(what.to_string());
}

// ---------------------------------------------------------------- helpers

fn parse_guid(s: &str) -> Result<Guid, ApiError> {
    s.parse().map_err(|_| ApiError::bad(format!("'{s}' is not a device GUID")))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn layout_for<'a>(layouts: &'a [DeviceLayout], d: &DeviceInfo) -> Option<&'a DeviceLayout> {
    layouts.iter().find(|l| l.matches(d))
}

/// Layout for a role: the resolved device's, else one sharing the role's USB IDs.
fn layout_for_role<'a>(layouts: &'a [DeviceLayout], profile: &Profile, role: &str, device: Option<&DeviceInfo>) -> Option<&'a DeviceLayout> {
    if let Some(d) = device {
        if let Some(l) = layout_for(layouts, d) {
            return Some(l);
        }
    }
    let r = profile.devices.get(role)?;
    layouts.iter().find(|l| {
        l.matches.iter().any(|lm| r.matches.iter().any(|rm| rm.vid.is_some() && rm.vid == lm.vid && rm.pid == lm.pid))
    })
}

impl AppState {
    fn snapshot(&self) -> Arc<Snapshot> {
        self.engine.snapshot()
    }

    fn device(&self, g: Guid) -> Option<DeviceInfo> {
        self.snapshot().devices.iter().find(|d| d.instance == g).cloned()
    }

    fn resolve(&self, p: &Profile) -> Vec<RoleResolution> {
        resolve_roles(p, &self.snapshot().devices, &self.store.assignments(&p.id))
    }

    fn profile(&self, id: &str) -> Result<Profile, ApiError> {
        self.store.profile(id).ok_or_else(|| ApiError::not_found("profile", id))
    }

    fn active_id(&self) -> Option<String> {
        self.snapshot().active.as_ref().map(|a| a.compiled.id.clone())
    }

    /// A device by GUID, or by role in `profile` (default: the active profile).
    fn device_ref(&self, text: &str, profile: Option<&str>) -> Result<Guid, ApiError> {
        if let Ok(g) = text.parse::<Guid>() {
            return Ok(g);
        }
        let pid = profile.map(str::to_string).or_else(|| self.active_id()).ok_or_else(|| {
            ApiError::bad(format!("'{text}' is not a device GUID, and there is no active profile to look the role up in"))
        })?;
        let p = self.profile(&pid)?;
        let res = self.resolve(&p);
        let r = res.iter().find(|r| r.role == text).ok_or_else(|| ApiError::bad(format!("profile '{pid}' has no role '{text}'")))?;
        r.device.ok_or_else(|| ApiError::new(StatusCode::CONFLICT, format!("role '{text}' has no device yet ({:?})", r.status)).with("role", r))
    }

    pub async fn activate(&self, id: &str) -> ApiResult {
        let p = self.profile(id)?;
        let compiled = p.compile().map_err(ApiError::invalid)?;
        let res = self.resolve(&p);
        if let Some(bad) = res.iter().find(|r| !r.is_ready()) {
            let msg = match bad.status {
                vbind_core::resolve::RoleStatus::Ambiguous => format!("more than one connected device fits role '{}'; assign one", bad.role),
                _ => format!("no connected device fits role '{}'", bad.role),
            };
            return Err(ApiError::new(StatusCode::CONFLICT, msg).with("roles", &res));
        }
        let roles = res.iter().map(|r| r.device).collect();
        self.engine
            .activate(ActivateSpec { compiled: Arc::new(compiled), roles })
            .await
            .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
        let _ = self.store.update_settings(|s| s.active_profile = Some(id.to_string()));
        *self.pending.lock().unwrap() = None;
        changed(self, "active");
        ok(json!({ "active": id, "roles": res }))
    }

    /// Keep the running profile's roles in step with plugged devices, and
    /// start the saved profile once its devices are present.
    pub async fn reconcile(&self) {
        let snap = self.snapshot();
        if let Some(a) = &snap.active {
            if let Some(p) = self.store.profile(&a.compiled.id) {
                let res = self.resolve(&p);
                let roles: Vec<Option<Guid>> = res.iter().map(|r| r.device).collect();
                if roles != a.roles {
                    self.engine.send(Command::UpdateRoles(roles));
                    changed(self, "active");
                }
            }
            return;
        }
        let Some(id) = self.store.settings().active_profile else {
            *self.pending.lock().unwrap() = None;
            return;
        };
        let pending = match self.activate(&id).await {
            Ok(_) => None,
            Err(e) => Some(Pending {
                profile: id.clone(),
                reason: e.body.get("error").and_then(Value::as_str).unwrap_or("could not start").to_string(),
                roles: e.body.get("roles").and_then(|r| serde_json::from_value(r.clone()).ok()).unwrap_or_default(),
            }),
        };
        let mut cur = self.pending.lock().unwrap();
        let differs = cur.as_ref().map(|c| (&c.reason, c.roles.len())) != pending.as_ref().map(|c| (&c.reason, c.roles.len()));
        *cur = pending;
        drop(cur);
        if differs {
            changed(self, "active");
        }
    }
}

// ---------------------------------------------------------------- status, docs

async fn status(State(st): State<Shared>) -> ApiResult {
    let snap = st.snapshot();
    let agent = st.agent.lock().unwrap();
    let agent_json = match (&agent.client, agent.at) {
        (Some(c), Some(at)) => json!({ "client": c, "seconds_ago": at.elapsed().as_secs() }),
        _ => Value::Null,
    };
    ok(json!({
        "version": VERSION,
        "data_dir": st.store.dir(),
        "port": st.port,
        "uptime_s": st.started.elapsed().as_secs(),
        "polling_hz": round3(snap.measured_hz),
        "devices": snap.devices.len(),
        "active_profile": snap.active.as_ref().map(|a| &a.compiled.id),
        "pending": *st.pending.lock().unwrap(),
        "outputs": {
            "xbox360": { "available": snap.vigem_error.is_none(), "error": snap.vigem_error },
            "vjoy": { "available": snap.vjoy_error.is_none(), "error": snap.vjoy_error, "devices": snap.vjoy_devices },
        },
        "input_error": snap.input_error,
        "agent": agent_json,
        "cli": cli_path(),
    }))
}

/// The `vbind` command-line tool agents use: next to this exe (the app ships
/// it as a sidecar), or this exe itself when running `vbind serve`.
pub fn cli_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidate = dir.join("vbind.exe");
    let path = if candidate.is_file() { candidate } else { exe };
    Some(path.display().to_string())
}

pub const GUIDE: &str = include_str!("../../../docs/AGENT_GUIDE.md");

async fn guide() -> Response {
    ([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], GUIDE).into_response()
}

pub fn schema_json(kind: &str) -> Option<Value> {
    let s = match kind {
        "profile" => schemars::schema_for!(Profile),
        "layout" => schemars::schema_for!(DeviceLayout),
        "panel" => schemars::schema_for!(PanelPost),
        _ => return None,
    };
    serde_json::to_value(s).ok()
}

async fn schema(Path(kind): Path<String>) -> ApiResult {
    let kind = kind.trim_end_matches(".json");
    schema_json(kind).map(Json).ok_or_else(|| ApiError::not_found("schema", kind))
}

// ---------------------------------------------------------------- devices and live state

fn device_json(st: &AppState, d: &DeviceInfo, layouts: &[DeviceLayout]) -> Value {
    let layout = layout_for(layouts, d);
    let snap = st.snapshot();
    let mut roles = Vec::new();
    if let Some(a) = &snap.active {
        for (i, r) in a.roles.iter().enumerate() {
            if *r == Some(d.instance) {
                roles.push(json!({ "profile": a.compiled.id, "role": a.compiled.roles[i] }));
            }
        }
    }
    json!({
        "instance": d.instance,
        "name": d.name,
        "vid": d.vid.map(|v| format!("{v:04X}")),
        "pid": d.pid.map(|v| format!("{v:04X}")),
        "axes": AXIS_NAMES.iter().zip(d.caps.axes).filter(|(_, p)| *p).map(|(n, _)| *n).collect::<Vec<_>>(),
        "buttons": d.caps.button_count,
        "hats": d.caps.hat_count,
        "layout": layout.map(|l| &l.id),
        "roles": roles,
    })
}

async fn devices(State(st): State<Shared>) -> ApiResult {
    let layouts = st.store.layouts();
    let snap = st.snapshot();
    ok(snap.devices.iter().map(|d| device_json(&st, d, &layouts)).collect::<Vec<_>>())
}

fn state_json(s: &DeviceState) -> Value {
    json!({
        "axes": s.axes.iter().map(|v| round3(*v)).collect::<Vec<_>>(),
        "hats": s.hats,
        "buttons": s.buttons.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i + 1).collect::<Vec<_>>(),
    })
}

fn report_json(r: &Report) -> Value {
    match r {
        Report::Xbox(x) => serde_json::to_value(x).unwrap_or(Value::Null),
        Report::Vjoy(v) => json!({
            "axes": v.axes.iter().map(|a| round3(*a)).collect::<Vec<_>>(),
            "buttons": v.buttons.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i + 1).collect::<Vec<_>>(),
            "hats": v.hats,
        }),
    }
}

pub fn live_json(st: &AppState, device_filter: Option<Guid>) -> Value {
    let snap = st.snapshot();
    let mut devices = serde_json::Map::new();
    for d in &snap.devices {
        if device_filter.is_some_and(|f| f != d.instance) {
            continue;
        }
        if let Some(s) = snap.states.get(&d.instance) {
            devices.insert(d.instance.to_string(), state_json(s));
        }
    }
    let active = snap.active.as_ref().map(|a| {
        let c = &a.compiled;
        json!({
            "profile": c.id,
            "name": c.name,
            "roles": c.roles.iter().zip(&a.roles).map(|(r, g)| (r.clone(), json!(g))).collect::<serde_json::Map<_, _>>(),
            "vars": c.vars.iter().zip(&a.vars).map(|(v, x)| (v.name.clone(), json!(round3(*x)))).collect::<serde_json::Map<_, _>>(),
            "values": c.mappings.iter().zip(&a.values).map(|(m, x)| (m.key.clone(), json!(round3(*x)))).collect::<serde_json::Map<_, _>>(),
            "outputs": a.outputs.iter().map(|o| {
                let mut v = serde_json::to_value(o).unwrap_or(Value::Null);
                if let Some(r) = &o.report { v["report"] = report_json(r); }
                v
            }).collect::<Vec<_>>(),
        })
    });
    json!({
        "seq": snap.seq,
        "time_ms": now_ms(),
        "polling_hz": round3(snap.measured_hz),
        "devices": devices,
        "active": active,
        "pending": *st.pending.lock().unwrap(),
        "capture": snap.capture.as_ref().map(|(p, t)| json!({ "prompt": p, "elapsed_ms": t.elapsed().as_millis() as u64 })),
        "simulated": snap.simulated,
    })
}

#[derive(Deserialize)]
struct StateQuery {
    device: Option<String>,
    profile: Option<String>,
}

async fn live_state(State(st): State<Shared>, Query(q): Query<StateQuery>) -> ApiResult {
    let filter = match &q.device {
        Some(d) => Some(st.device_ref(d, q.profile.as_deref())?),
        None => None,
    };
    Ok(Json(live_json(&st, filter)))
}

#[derive(Deserialize)]
struct EventsQuery {
    #[serde(default)]
    hz: Option<u32>,
}

/// Server-sent events: `state` (live values) at up to 30 Hz, and `changed`
/// (`profiles`, `layouts`, `panel`, `devices`, `active`) when something changes.
async fn events(State(st): State<Shared>, Query(q): Query<EventsQuery>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let hz = q.hz.unwrap_or(30).clamp(1, 60);
    let interval = Duration::from_millis(1000 / hz as u64);
    let st2 = st.clone();
    let ticks = stream::unfold((st2, 0u64, u64::MAX), move |(st, last_seq, last_rev)| async move {
        tokio::time::sleep(interval).await;
        let snap = st.snapshot();
        let rev = st.engine.shared.devices_rev.load(std::sync::atomic::Ordering::SeqCst);
        let mut events = Vec::new();
        if rev != last_rev {
            events.push(Event::default().event("changed").data("devices"));
        }
        if snap.seq != last_seq {
            events.push(Event::default().event("state").data(live_json(&st, None).to_string()));
        }
        Some((stream::iter(events.into_iter().map(Ok)), (st, snap.seq, rev)))
    })
    .flatten();
    let rx = st.changes.subscribe();
    let changes = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(what) => return Some((Ok(Event::default().event("changed").data(what)), rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream::select(ticks, changes)).keep_alive(KeepAlive::default())
}

// ---------------------------------------------------------------- capture, simulate, eval

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureBody {
    #[serde(default)]
    prompt: Option<String>,
    /// Any of "button", "hat", "axis". Default: all three.
    #[serde(default)]
    kinds: Option<Vec<String>>,
    /// Only listen to this device (GUID or role).
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    timeout_s: Option<f64>,
}

async fn capture(State(st): State<Shared>, Json(b): Json<CaptureBody>) -> ApiResult {
    let kinds = b.kinds.unwrap_or_else(|| vec!["button".into(), "hat".into(), "axis".into()]);
    for k in &kinds {
        if !["button", "hat", "axis"].contains(&k.as_str()) {
            return Err(ApiError::bad(format!("unknown capture kind '{k}'; use button, hat or axis")));
        }
    }
    let devices = match &b.device {
        Some(d) => Some(vec![st.device_ref(d, b.profile.as_deref())?]),
        None => None,
    };
    let prompt = b.prompt.unwrap_or_else(|| "Press a button or move a control".into());
    let timeout = Duration::from_secs_f64(b.timeout_s.unwrap_or(20.0).clamp(1.0, 300.0));
    changed(&st, "capture");
    let result = st
        .engine
        .capture(CaptureRequest {
            prompt,
            buttons: kinds.iter().any(|k| k == "button"),
            hats: kinds.iter().any(|k| k == "hat"),
            axes: kinds.iter().any(|k| k == "axis"),
            devices,
            timeout,
            axis_threshold: 0.5,
        })
        .await;
    changed(&st, "capture");
    let captured = result.map_err(|e| ApiError::new(StatusCode::CONFLICT, e))?;
    let Some(c) = captured else {
        return ok(json!({ "captured": null, "timed_out": true }));
    };
    // Add the layout label and any role this device fills.
    let layouts = st.store.layouts();
    let info = st.device(c.device);
    let segs: Vec<&str> = c.control.split('.').collect();
    let control = InputControl::parse(&segs).ok();
    let label = info
        .as_ref()
        .and_then(|d| layout_for(&layouts, d))
        .zip(control.as_ref())
        .and_then(|(l, ctl)| l.label(ctl))
        .or_else(|| control.as_ref().map(vbind_core::graph::default_input_label));
    let mut roles = Vec::new();
    for sp in st.store.profiles().iter().filter_map(|s| s.profile.as_ref()) {
        for r in st.resolve(sp) {
            if r.device == Some(c.device) {
                roles.push(json!({ "profile": sp.id, "role": r.role, "expr": format!("{}.{}", r.role, c.control) }));
            }
        }
    }
    ok(json!({ "captured": c, "label": label, "roles": roles, "timed_out": false }))
}

async fn cancel_capture(State(st): State<Shared>) -> ApiResult {
    st.engine.send(Command::CancelCapture);
    ok(json!({ "cancelled": true }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SimulateBody {
    /// Device GUID or role name.
    device: String,
    #[serde(default)]
    profile: Option<String>,
    /// Axis name to value, `{"y": -1}`.
    #[serde(default)]
    axes: IndexMap<String, f64>,
    /// Button to pressed, `{"b1": true}`.
    #[serde(default)]
    buttons: IndexMap<String, bool>,
    /// Hat to direction or null, `{"hat1": "up"}`.
    #[serde(default)]
    hats: IndexMap<String, Option<String>>,
    /// How long to hold it. Default 3 seconds, max 600.
    #[serde(default)]
    duration_s: Option<f64>,
}

async fn simulate(State(st): State<Shared>, Json(b): Json<SimulateBody>) -> ApiResult {
    let device = st.device_ref(&b.device, b.profile.as_deref())?;
    if st.device(device).is_none() {
        return Err(ApiError::new(StatusCode::CONFLICT, format!("device {device} is not connected")));
    }
    let mut sim = Simulation { device, ..Default::default() };
    for (name, v) in &b.axes {
        let i = AXIS_NAMES.iter().position(|a| a == name).ok_or_else(|| ApiError::bad(format!("unknown axis '{name}'")))?;
        sim.axes.push((i, v.clamp(-1.0, 1.0)));
    }
    for (name, v) in &b.buttons {
        let n: usize = name.trim_start_matches('b').parse().map_err(|_| ApiError::bad(format!("buttons are b1..b128, not '{name}'")))?;
        if !(1..=128).contains(&n) {
            return Err(ApiError::bad("buttons are b1..b128"));
        }
        sim.buttons.push((n - 1, *v));
    }
    for (name, dir) in &b.hats {
        let n: usize = name.trim_start_matches("hat").parse().unwrap_or(1);
        if !(1..=4).contains(&n) {
            return Err(ApiError::bad("hats are hat1..hat4"));
        }
        let angle = match dir {
            None => None,
            Some(d) => Some(expr::HatDir::from_name(d).ok_or_else(|| ApiError::bad(format!("unknown hat direction '{d}'")))?.angle()),
        };
        sim.hats.push((n - 1, angle));
    }
    let secs = b.duration_s.unwrap_or(3.0).clamp(0.05, 600.0);
    sim.until = Some(Instant::now() + Duration::from_secs_f64(secs));
    st.engine.send(Command::Simulate(sim));
    ok(json!({ "device": device, "for_s": secs }))
}

async fn clear_simulation(State(st): State<Shared>) -> ApiResult {
    st.engine.send(Command::ClearSimulation(None));
    ok(json!({ "cleared": true }))
}

async fn xinput() -> ApiResult {
    ok(vbind_io::xinput::read_all())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalBody {
    expr: String,
    #[serde(default)]
    profile: Option<String>,
}

struct EvalScope<'a>(&'a Profile);
impl Scope for EvalScope<'_> {
    fn role(&self, name: &str) -> Option<usize> {
        self.0.devices.get_index_of(name)
    }
    fn var(&self, name: &str) -> Option<usize> {
        self.0.vars.get_index_of(name)
    }
    fn role_names(&self) -> Vec<&str> {
        self.0.devices.keys().map(String::as_str).collect()
    }
    fn var_names(&self) -> Vec<&str> {
        self.0.vars.keys().map(String::as_str).collect()
    }
}

/// Evaluate an expression against live input, using a profile's roles and variables.
async fn eval_expr(State(st): State<Shared>, Json(b): Json<EvalBody>) -> ApiResult {
    let pid = b.profile.clone().or_else(|| st.active_id()).ok_or_else(|| ApiError::bad("give a profile (no profile is active)"))?;
    let p = st.profile(&pid)?;
    let compiled = p.compile().map_err(ApiError::invalid)?;
    let scope = EvalScope(&p);
    let node = expr::parse(&b.expr)
        .and_then(|a| Resolver::new(&scope).resolve(&a))
        .map_err(|e| ApiError::bad(e.to_string()).with("pos", e.pos))?;
    let snap = st.snapshot();
    let res = st.resolve(&p);
    let devs: Vec<Option<&DeviceState>> = res.iter().map(|r| r.device.and_then(|g| snap.states.get(&g))).collect();
    let vars = match &snap.active {
        Some(a) if a.compiled.id == pid => a.vars.clone(),
        _ => {
            let mut rt = compiled.runtime();
            compiled.evaluate(&devs, &mut rt);
            rt.vars
        }
    };
    let value = expr::eval(&node, &Env { devices: &devs, vars: &vars }, &mut EvalState::new(64));
    ok(json!({ "value": round3(value), "truthy": expr::truthy(value), "profile": pid }))
}

// ---------------------------------------------------------------- profiles

async fn list_profiles(State(st): State<Shared>) -> ApiResult {
    let active = st.active_id();
    let pending = st.pending.lock().unwrap().clone();
    let list: Vec<Value> = st
        .store
        .profiles()
        .iter()
        .map(|sp| match &sp.profile {
            Some(p) => {
                let errors = p.compile().err().unwrap_or_default();
                json!({
                    "id": p.id, "name": p.name, "game": p.game, "description": p.description,
                    "outputs": p.outputs.values().map(|o| o.kind).collect::<Vec<_>>(),
                    "valid": errors.is_empty(), "errors": errors,
                    "active": active.as_deref() == Some(p.id.as_str()),
                    "pending": pending.as_ref().filter(|x| x.profile == p.id).map(|x| &x.reason),
                    "roles": st.resolve(p),
                    "role_labels": p.devices.iter().map(|(k, d)| (k.clone(), json!(d.label.clone().unwrap_or_else(|| k.clone())))).collect::<serde_json::Map<_, _>>(),
                })
            }
            None => json!({ "id": sp.id, "valid": false, "errors": [sp.load_error], "active": false }),
        })
        .collect();
    ok(list)
}

async fn get_profile(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    ok(st.profile(&id)?)
}

fn parse_profile(v: Value) -> Result<Profile, ApiError> {
    serde_json::from_value::<Profile>(v).map_err(|e| ApiError::invalid(vec![Diagnostic { path: "(file)".into(), message: e.to_string(), pos: None }]))
}

async fn save(st: &AppState, p: Profile) -> ApiResult {
    p.compile().map_err(ApiError::invalid)?;
    st.store.save_profile(&p).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    changed(st, "profiles");
    let mut reloaded = false;
    if st.active_id().as_deref() == Some(p.id.as_str()) {
        let _ = st.activate(&p.id).await?;
        reloaded = true;
    }
    ok(json!({ "saved": p.id, "reloaded": reloaded, "roles": st.resolve(&p) }))
}

#[derive(Deserialize)]
struct ImportQuery {
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    activate: bool,
}

/// Import (create) a profile. Fails if the id exists unless `?replace=true`.
async fn import_profile(State(st): State<Shared>, Query(q): Query<ImportQuery>, Json(v): Json<Value>) -> ApiResult {
    let p = parse_profile(v)?;
    if st.store.profile_exists(&p.id) && !q.replace {
        return Err(ApiError::new(StatusCode::CONFLICT, format!("profile '{}' already exists; pass replace=true to overwrite", p.id)));
    }
    let id = p.id.clone();
    let res = save(&st, p).await?;
    if q.activate {
        return st.activate(&id).await;
    }
    Ok(res)
}

async fn put_profile(State(st): State<Shared>, Path(id): Path<String>, Json(v): Json<Value>) -> ApiResult {
    let p = parse_profile(v)?;
    if p.id != id {
        return Err(ApiError::bad(format!("profile id '{}' does not match the URL '{id}'", p.id)));
    }
    save(&st, p).await
}

async fn delete_profile(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    if st.active_id().as_deref() == Some(id.as_str()) {
        st.engine.deactivate().await;
    }
    if st.store.settings().active_profile.as_deref() == Some(id.as_str()) {
        let _ = st.store.update_settings(|s| s.active_profile = None);
    }
    let removed = st.store.delete_profile(&id).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !removed {
        return Err(ApiError::not_found("profile", &id));
    }
    changed(&st, "profiles");
    ok(json!({ "deleted": id }))
}

/// Check a profile without saving it. Returns diagnostics and, when valid, the graph.
async fn validate_profile(State(st): State<Shared>, Json(v): Json<Value>) -> ApiResult {
    let p = match serde_json::from_value::<Profile>(v) {
        Ok(p) => p,
        Err(e) => return ok(json!({ "valid": false, "diagnostics": [Diagnostic { path: "(file)".into(), message: e.to_string(), pos: None }] })),
    };
    match p.compile() {
        Ok(c) => {
            let g = graph_for(&st, &p, &c);
            ok(json!({ "valid": true, "diagnostics": [], "roles": st.resolve(&p), "graph": g }))
        }
        Err(d) => ok(json!({ "valid": false, "diagnostics": d })),
    }
}

async fn activate(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    st.activate(&id).await
}

async fn deactivate(State(st): State<Shared>) -> ApiResult {
    st.engine.deactivate().await;
    let _ = st.store.update_settings(|s| s.active_profile = None);
    *st.pending.lock().unwrap() = None;
    changed(&st, "active");
    ok(json!({ "active": null }))
}

fn graph_for(st: &AppState, p: &Profile, c: &vbind_core::CompiledProfile) -> vbind_core::graph::Graph {
    let layouts = st.store.layouts();
    let res = st.resolve(p);
    let snap = st.snapshot();
    let label = |role: usize, ctl: &InputControl| -> Option<String> {
        let name = c.roles.get(role)?;
        let dev = res.get(role).and_then(|r| r.device).and_then(|g| snap.devices.iter().find(|d| d.instance == g));
        layout_for_role(&layouts, p, name, dev).and_then(|l| l.label(ctl))
    };
    build_graph(p, c, &label)
}

async fn graph(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let p = st.profile(&id)?;
    let c = p.compile().map_err(ApiError::invalid)?;
    let g = graph_for(&st, &p, &c);
    let res = st.resolve(&p);
    let snap = st.snapshot();
    let layouts = st.store.layouts();
    let devices: Vec<Value> = res
        .iter()
        .map(|r| {
            let d = r.device.and_then(|g| snap.devices.iter().find(|d| d.instance == g));
            json!({
                "role": r.role,
                "resolution": r,
                "device": d.map(|d| json!({ "instance": d.instance, "name": d.name })),
                "layout": layout_for_role(&layouts, &p, &r.role, d).map(|l| &l.id),
            })
        })
        .collect();
    ok(json!({ "graph": g, "devices": devices }))
}

async fn roles(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let p = st.profile(&id)?;
    ok(st.resolve(&p))
}

/// `{"left": "<guid>", "right": null}`: pick (or clear) the device for roles.
async fn assign(State(st): State<Shared>, Path(id): Path<String>, Json(body): Json<IndexMap<String, Option<String>>>) -> ApiResult {
    let p = st.profile(&id)?;
    for (role, dev) in &body {
        if !p.devices.contains_key(role) {
            return Err(ApiError::bad(format!("profile '{id}' has no role '{role}'")));
        }
        let g = match dev {
            Some(d) => Some(parse_guid(d)?),
            None => None,
        };
        st.store.set_assignment(&id, role, g).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }
    changed(&st, "profiles");
    if st.active_id().as_deref() == Some(id.as_str()) {
        let _ = st.activate(&id).await?;
    }
    ok(json!({ "roles": st.resolve(&p) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapEdit {
    /// Output key, `pad.a`.
    key: String,
    /// New expression; null removes the mapping.
    #[serde(default)]
    expr: Option<String>,
    /// New game-action label; empty string removes it.
    #[serde(default)]
    label: Option<String>,
}

/// Change one mapping (used by the inspector in the window).
async fn edit_map(State(st): State<Shared>, Path(id): Path<String>, Json(e): Json<MapEdit>) -> ApiResult {
    let mut p = st.profile(&id)?;
    match &e.expr {
        Some(x) => {
            p.map.insert(e.key.clone(), x.clone());
        }
        None => {
            p.map.shift_remove(&e.key);
        }
    }
    if let Some(l) = &e.label {
        if l.is_empty() {
            p.labels.shift_remove(&e.key);
        } else {
            p.labels.insert(e.key.clone(), l.clone());
        }
    }
    save(&st, p).await
}

// ---------------------------------------------------------------- layouts

async fn list_layouts(State(st): State<Shared>) -> ApiResult {
    ok(st.store.layouts())
}

async fn get_layout(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    st.store.layouts().into_iter().find(|l| l.id == id).map(|l| Json(serde_json::to_value(l).unwrap())).ok_or_else(|| ApiError::not_found("layout", &id))
}

async fn store_layout(st: &AppState, v: Value) -> ApiResult {
    let l: DeviceLayout = serde_json::from_value(v).map_err(|e| ApiError::invalid(vec![Diagnostic { path: "(file)".into(), message: e.to_string(), pos: None }]))?;
    let errs = l.validate();
    if !errs.is_empty() {
        return Err(ApiError::invalid(errs));
    }
    st.store.save_layout(&l).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    changed(st, "layouts");
    ok(json!({ "saved": l.id }))
}

async fn save_layout_post(State(st): State<Shared>, Json(v): Json<Value>) -> ApiResult {
    store_layout(&st, v).await
}

async fn save_layout_put(State(st): State<Shared>, Path(id): Path<String>, Json(v): Json<Value>) -> ApiResult {
    if v.get("id").and_then(Value::as_str) != Some(id.as_str()) {
        return Err(ApiError::bad("layout id does not match the URL"));
    }
    store_layout(&st, v).await
}

async fn delete_layout(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let removed = st.store.delete_layout(&id).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !removed {
        return Err(ApiError::not_found("layout", &id));
    }
    changed(&st, "layouts");
    ok(json!({ "deleted": id }))
}

// ---------------------------------------------------------------- panel

async fn get_panel(State(st): State<Shared>) -> ApiResult {
    ok(st.panel.get())
}

async fn post_panel(State(st): State<Shared>, Json(post): Json<PanelPost>) -> ApiResult {
    let p = st.panel.post(post);
    changed(&st, "panel");
    ok(p)
}

async fn clear_panel(State(st): State<Shared>) -> ApiResult {
    st.panel.clear();
    changed(&st, "panel");
    ok(json!({ "cleared": true }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskBody {
    question: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    allow_text: bool,
    #[serde(default)]
    timeout_s: Option<f64>,
}

/// Show a question in the window and wait for the answer.
async fn ask(State(st): State<Shared>, Json(b): Json<AskBody>) -> ApiResult {
    if b.options.is_empty() && !b.allow_text {
        return Err(ApiError::bad("give options to pick from, or allow_text"));
    }
    let timeout = Duration::from_secs_f64(b.timeout_s.unwrap_or(120.0).clamp(5.0, 900.0));
    changed(&st, "panel");
    let (id, answer) = st.panel.ask(b.question, b.options, b.allow_text, timeout).await;
    changed(&st, "panel");
    ok(json!({ "id": id, "answer": answer, "timed_out": answer.is_none() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerBody {
    id: String,
    answer: String,
}

async fn answer(State(st): State<Shared>, Json(b): Json<AnswerBody>) -> ApiResult {
    if !st.panel.answer(&b.id, b.answer) {
        return Err(ApiError::not_found("question", &b.id));
    }
    changed(&st, "panel");
    ok(json!({ "answered": b.id }))
}

// ---------------------------------------------------------------- bridge import, reload

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct BridgeImport {
    /// Path to hosas_config.json. Default: %LOCALAPPDATA%\GalacticRacerHOSAS\hosas_config.json
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    /// Save the profile and its device assignments (default true).
    #[serde(default)]
    save: Option<bool>,
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    activate: bool,
}

pub fn default_bridge_config() -> std::path::PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_default();
    std::path::Path::new(&base).join("GalacticRacerHOSAS").join("hosas_config.json")
}

async fn import_bridge(State(st): State<Shared>, body: Option<Json<BridgeImport>>) -> ApiResult {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let path = b.path.map(std::path::PathBuf::from).unwrap_or_else(default_bridge_config);
    let text = std::fs::read_to_string(&path).map_err(|e| ApiError::not_found("bridge config", &format!("{} ({e})", path.display())))?;
    let cfg: Value = serde_json::from_str(&text).map_err(|e| ApiError::bad(format!("{} is not valid JSON: {e}", path.display())))?;
    let mut opts = import_hosas::Options::default();
    if let Some(id) = b.id {
        opts.id = id;
    }
    if let Some(n) = b.name {
        opts.name = n;
    }
    let lookup = |g: Guid| st.device(g);
    let conv = import_hosas::convert(&cfg, &lookup, &opts).map_err(ApiError::bad)?;
    let mut result = json!({ "profile": conv.profile, "warnings": conv.warnings, "assignments": conv.assignments.iter().map(|(r, g)| (r.clone(), json!(g))).collect::<serde_json::Map<_, _>>(), "source": path });
    if b.save.unwrap_or(true) {
        let id = conv.profile.id.clone();
        if st.store.profile_exists(&id) && !b.replace {
            return Err(ApiError::new(StatusCode::CONFLICT, format!("profile '{id}' already exists; pass replace=true to overwrite")).with("profile", &conv.profile));
        }
        for (role, g) in &conv.assignments {
            st.store.set_assignment(&id, role, Some(*g)).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        }
        let _ = save(&st, conv.profile).await?;
        result["saved"] = json!(id);
        if b.activate {
            let _ = st.activate(&id).await?;
            result["active"] = json!(id);
        }
    }
    Ok(Json(result))
}

async fn reload(State(st): State<Shared>) -> ApiResult {
    st.store.reload();
    changed(&st, "profiles");
    changed(&st, "layouts");
    ok(json!({ "reloaded": true }))
}

// ---------------------------------------------------------------- the window's files

async fn ui_asset(State(st): State<Shared>, req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let file = UiAssets::get(path).map(|f| (path.to_string(), f)).or_else(|| {
        // Single-page app: unknown paths without an extension get the page.
        (!path.contains('.')).then(|| UiAssets::get("index.html").map(|f| ("index.html".to_string(), f))).flatten()
    });
    let Some((name, file)) = file else {
        if path == "index.html" {
            return (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                "<!doctype html><title>VBind</title><body style='font-family:sans-serif;background:#1c1a19;color:#f5f1ee;padding:40px'><h1>VBind is running</h1><p>The window's files are not built. Run <code>npm --prefix ui run build</code>.</p>",
            )
                .into_response();
        }
        return StatusCode::NOT_FOUND.into_response();
    };
    let mime = mime_guess::from_path(&name).first_or_octet_stream();
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    if name == "index.html" {
        // Same-origin pages can read this; other sites cannot (no CORS).
        let html = String::from_utf8_lossy(&file.data).replace(
            "</head>",
            &format!("<script>window.VBIND_TOKEN={};</script></head>", serde_json::to_string(&st.token).unwrap()),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        return (headers, html).into_response();
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    (headers, Body::from(file.data.into_owned())).into_response()
}
