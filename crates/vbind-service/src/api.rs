//! The local HTTP API on 127.0.0.1. The window, the `vbind` CLI and the MCP
//! server all use it. Every `/api` call needs the token from runtime.json.

use crate::engine::{ActivateSpec, CaptureRequest, Command, EngineHandle, Simulation, Snapshot};
use crate::import_hosas;
use crate::panel::{Panel, PanelPost};
use crate::store::Store;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
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
use vbind_core::drawing::{ResolvedView, View};
use vbind_core::layout::{ControlInfo, DeviceLayout};
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
        .route("/profiles/{id}/hints", put(set_hints))
        .route("/deactivate", post(deactivate))
        .route("/layouts", get(list_layouts).post(save_layout_post))
        .route("/layouts/{id}", get(get_layout).put(save_layout_put).delete(delete_layout))
        .route("/layouts/{id}/drawing", get(layout_drawing))
        .route("/layouts/{id}/render", get(render_layout))
        .route("/layouts/{id}/place", post(place_controls))
        .route("/layouts/{id}/views/{view}", put(put_view).delete(delete_view))
        .route(
            "/layouts/{id}/views/{view}/photo",
            get(get_photo).put(put_photo).delete(delete_photo).layer(DefaultBodyLimit::max(crate::photos::MAX_PHOTO_BYTES * 4 / 3 + 64 * 1024)),
        )
        .route("/layout-kit", get(layout_kit))
        .route("/panel", get(get_panel).post(post_panel).delete(clear_panel))
        .route("/panel/ask", post(ask))
        .route("/panel/answer", post(answer))
        .route("/integrations", get(list_integrations).post(import_integration))
        .route("/integrations/{id}", get(get_integration).put(put_integration).delete(delete_integration))
        .route("/integrations/{id}/install", post(install_integration))
        .route("/integrations/{id}/uninstall", post(uninstall_integration))
        .route("/integrations/{id}/launch", post(launch_integration))
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

    /// Compile against the integrations installed on this PC.
    pub fn compile(&self, p: &Profile) -> Result<vbind_core::CompiledProfile, Vec<Diagnostic>> {
        p.compile_with(&self.store.integrations.defs())
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
        let compiled = self.compile(&p).map_err(ApiError::invalid)?;
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
            "keyboard": { "available": true, "error": null },
        },
        "input_error": snap.input_error,
        "integrations": st.store.integrations.list().iter().map(|i| json!({ "id": i.id, "valid": i.def.is_some() })).collect::<Vec<_>>(),
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
        "integration" => schemars::schema_for!(vbind_core::integration::Integration),
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

fn report_json(r: &Report, out: &vbind_core::profile::CompiledOutput) -> Value {
    match r {
        Report::Integration { controls, settings } => {
            let Some(def) = out.integration.as_ref() else { return json!({}) };
            let mut obj: serde_json::Map<String, Value> =
                def.controls.keys().cloned().zip(controls).map(|(k, v)| (k, json!(round3(*v)))).collect();
            // mapped settings show the value the game gets, for example "DirectThrust"
            for &i in &out.live_settings {
                if let (Some((id, s)), Some(v)) = (def.settings.get_index(i), settings.get(i)) {
                    obj.insert(format!("{}{id}", vbind_core::profile::SETTINGS_PREFIX), vbind_core::integration::setting_from_number(s, *v));
                }
            }
            Value::Object(obj)
        }
        Report::Xbox(x) => serde_json::to_value(x).unwrap_or(Value::Null),
        Report::Keyboard(k) => json!({
            "held": k.held(),
            "mouse": { "x": round3(k.mouse[0]), "y": round3(k.mouse[1]), "wheel": round3(k.mouse[2]) },
        }),
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
            "outputs": a.outputs.iter().zip(&c.outputs).map(|(o, co)| {
                let mut v = serde_json::to_value(o).unwrap_or(Value::Null);
                if let Some(r) = &o.report { v["report"] = report_json(r, co); }
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
    let compiled = st.compile(&p).map_err(ApiError::invalid)?;
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
                let errors = st.compile(p).err().unwrap_or_default();
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
    st.compile(&p).map_err(ApiError::invalid)?;
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
    match st.compile(&p) {
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
    let c = st.compile(&p).map_err(ApiError::invalid)?;
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

/// `{"pad.a": "Index finger, like a gas pedal", "pad.b": null}`: set or clear memory hints.
async fn set_hints(State(st): State<Shared>, Path(id): Path<String>, Json(body): Json<IndexMap<String, Option<String>>>) -> ApiResult {
    let mut p = st.profile(&id)?;
    for (key, hint) in body {
        match hint.map(|h| h.trim().to_string()).filter(|h| !h.is_empty()) {
            Some(h) => {
                p.hints.insert(key, h);
            }
            None => {
                p.hints.shift_remove(&key);
            }
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
    st.store.photos().delete_all(&id);
    changed(&st, "layouts");
    ok(json!({ "deleted": id }))
}

fn find_layout(st: &AppState, id: &str) -> Result<DeviceLayout, ApiError> {
    st.store.layouts().into_iter().find(|l| l.id == id).ok_or_else(|| ApiError::not_found("layout", id))
}

fn find_view<'a>(l: &'a DeviceLayout, view: &str) -> Result<&'a View, ApiError> {
    l.views.iter().find(|v| v.id == view).ok_or_else(|| {
        let ids = l.views.iter().map(|v| v.id.as_str()).collect::<Vec<_>>().join(", ");
        ApiError::not_found("view", view).with("views", if ids.is_empty() { "(none yet)".to_string() } else { ids })
    })
}

/// Validate and save a changed layout.
fn save_changed_layout(st: &AppState, l: &DeviceLayout) -> Result<(), ApiError> {
    let errs = l.validate();
    if !errs.is_empty() {
        return Err(ApiError::invalid(errs));
    }
    st.store.save_layout(l).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    changed(st, "layouts");
    Ok(())
}

/// The kit of generic parts views are drawn from.
async fn layout_kit() -> ApiResult {
    ok(vbind_core::drawing::kit_list())
}

/// A layout's views as plain shapes, ready to draw, with any photos.
async fn layout_drawing(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let l = find_layout(&st, &id)?;
    let photos = st.store.photos().list(&id);
    let views: Vec<Value> = l
        .views
        .iter()
        .map(|v| {
            let r: ResolvedView = v.resolve();
            let mut j = serde_json::to_value(r).unwrap_or(Value::Null);
            if let Some(p) = photos.get(&v.id) {
                j["photo"] = json!({ "fit": p.fit, "width": p.width, "height": p.height, "rev": p.rev, "url": format!("/api/layouts/{id}/views/{}/photo?rev={}", v.id, p.rev) });
            }
            j
        })
        .collect();
    ok(json!({ "id": l.id, "name": l.name, "shape": l.shape, "views": views }))
}

#[derive(Deserialize)]
struct RenderQuery {
    #[serde(default)]
    view: Option<String>,
    /// Draw the photo under the outlines.
    #[serde(default)]
    photo: bool,
    #[serde(default)]
    size: Option<f64>,
    #[serde(default)]
    grid: Option<bool>,
    /// Comma-separated control ids to fill in.
    #[serde(default)]
    highlight: Option<String>,
}

/// A PNG of one view with a grid and numbered controls, for agents to check their drawing.
async fn render_layout(State(st): State<Shared>, Path(id): Path<String>, Query(q): Query<RenderQuery>) -> Result<Response, ApiError> {
    let l = find_layout(&st, &id)?;
    let view = match &q.view {
        Some(v) => find_view(&l, v)?.clone(),
        None => l.views.first().cloned().ok_or_else(|| ApiError::bad(format!("layout '{id}' has no views yet; add one with set_layout_view")))?,
    };
    let photos = st.store.photos();
    let photo = if q.photo {
        let info = photos.list(&id).shift_remove(&view.id).ok_or_else(|| ApiError::bad(format!("view '{}' has no photo", view.id)))?;
        let (bytes, _) = photos.read(&id, &view.id).ok_or_else(|| ApiError::not_found("photo", &view.id))?;
        Some((info, bytes))
    } else {
        None
    };
    let opts = crate::render::RenderOptions {
        size_px: q.size.unwrap_or(760.0).clamp(200.0, 2000.0),
        grid: q.grid.unwrap_or(true),
        highlight: q.highlight.map(|h| h.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default(),
    };
    let png = tokio::task::spawn_blocking(move || {
        let svg = crate::render::view_svg(&l, &view, photo.as_ref().map(|(i, b)| (i, b.as_slice())), &opts);
        crate::render::png(&svg)
    })
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "no-store")], png).into_response())
}

/// Create or replace one view. The body is the view; its id may be left out.
async fn put_view(State(st): State<Shared>, Path((id, view)): Path<(String, String)>, Json(mut v): Json<Value>) -> ApiResult {
    let mut l = find_layout(&st, &id)?;
    match v.get("id").and_then(Value::as_str) {
        Some(given) if given != view => return Err(ApiError::bad("view id does not match the URL")),
        Some(_) => {}
        None => {
            if let Some(o) = v.as_object_mut() {
                o.insert("id".into(), json!(view));
            }
        }
    }
    let new: View = serde_json::from_value(v).map_err(|e| ApiError::invalid(vec![Diagnostic { path: format!("views.{view}"), message: e.to_string(), pos: None }]))?;
    match l.views.iter_mut().find(|x| x.id == view) {
        Some(slot) => *slot = new,
        None => l.views.push(new),
    }
    save_changed_layout(&st, &l)?;
    let placed = l.controls.values().filter(|c| c.at.is_some() && l.view_of(c).is_some_and(|x| x.id == view)).count();
    ok(json!({ "saved": id, "view": view, "views": l.views.iter().map(|v| &v.id).collect::<Vec<_>>(), "controls_on_view": placed }))
}

/// Remove a view, its photo, and the placements of controls drawn on it.
async fn delete_view(State(st): State<Shared>, Path((id, view)): Path<(String, String)>) -> ApiResult {
    let mut l = find_layout(&st, &id)?;
    find_view(&l, &view)?;
    let single = l.views.len() == 1;
    for c in l.controls.values_mut() {
        if c.view.as_deref() == Some(view.as_str()) || (single && c.view.is_none()) {
            c.view = None;
            c.at = None;
        }
    }
    l.views.retain(|v| v.id != view);
    save_changed_layout(&st, &l)?;
    let _ = st.store.photos().delete(&id, &view);
    ok(json!({ "deleted": view }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Placement {
    #[serde(default)]
    at: Option<[f64; 2]>,
    #[serde(default)]
    view: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    size: Option<f64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PlaceValue {
    At([f64; 2]),
    Full(Placement),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceBody {
    /// View for every control that does not name its own.
    #[serde(default)]
    view: Option<String>,
    /// Control id to `[x, y]`, to `{at, view, label, size}`, or to null to take it off the picture.
    controls: IndexMap<String, Option<PlaceValue>>,
}

/// Move controls on a layout's views (also adds controls when given a label).
async fn place_controls(State(st): State<Shared>, Path(id): Path<String>, Json(b): Json<PlaceBody>) -> ApiResult {
    let mut l = find_layout(&st, &id)?;
    if let Some(v) = &b.view {
        find_view(&l, v)?;
    }
    for (key, value) in b.controls {
        let Some(value) = value else {
            if let Some(c) = l.controls.get_mut(&key) {
                c.at = None;
                c.view = None;
            }
            continue;
        };
        let p = match value {
            PlaceValue::At(at) => Placement { at: Some(at), view: None, label: None, size: None },
            PlaceValue::Full(p) => p,
        };
        if !l.controls.contains_key(&key) {
            let label = p.label.clone().ok_or_else(|| ApiError::bad(format!("'{key}' is not in this layout yet; give it a label to add it")))?;
            l.controls.insert(key.clone(), ControlInfo::new(label));
        }
        let entry = l.controls.get_mut(&key).expect("just inserted");
        if let Some(label) = p.label {
            entry.label = label;
        }
        if let Some(at) = p.at {
            entry.at = Some(at);
            entry.view = p.view.or_else(|| b.view.clone()).or_else(|| entry.view.clone());
        } else if let Some(v) = p.view {
            entry.view = Some(v);
        }
        if p.size.is_some() {
            entry.size = p.size;
        }
    }
    save_changed_layout(&st, &l)?;
    ok(json!({ "saved": id, "controls": l.controls }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PhotoBody {
    /// A PNG, JPEG or WebP file on this PC.
    #[serde(default)]
    path: Option<String>,
    /// The image itself, base64 encoded (the window uses this).
    #[serde(default)]
    data: Option<String>,
    /// Where the photo sits in the view: `[x, y, width, height]` in view units. Default: fitted inside the view.
    #[serde(default)]
    fit: Option<[f64; 4]>,
}

/// Add or replace a view's photo, or move it with only `fit`.
async fn put_photo(State(st): State<Shared>, Path((id, view)): Path<(String, String)>, Json(b): Json<PhotoBody>) -> ApiResult {
    let l = find_layout(&st, &id)?;
    let size = find_view(&l, &view)?.size;
    let bytes = match (&b.path, &b.data) {
        (Some(_), Some(_)) => return Err(ApiError::bad("give path or data, not both")),
        (Some(path), None) => Some(std::fs::read(path).map_err(|e| ApiError::bad(format!("cannot read {path}: {e}")))?),
        (None, Some(data)) => {
            use base64::Engine as _;
            let data = data.split_once("base64,").map(|(_, d)| d).unwrap_or(data);
            Some(base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| ApiError::bad(format!("data is not base64: {e}")))?)
        }
        (None, None) => None,
    };
    let info = st.store.photos().set(&id, &view, size, bytes.as_deref(), b.fit).map_err(ApiError::bad)?;
    changed(&st, "layouts");
    ok(json!({ "saved": view, "photo": { "fit": info.fit, "width": info.width, "height": info.height }, "view_size": size }))
}

async fn get_photo(State(st): State<Shared>, Path((id, view)): Path<(String, String)>) -> Result<Response, ApiError> {
    let (bytes, mime) = st.store.photos().read(&id, &view).ok_or_else(|| ApiError::not_found("photo", &format!("{id}/{view}")))?;
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "private, max-age=31536000".to_string())], bytes).into_response())
}

async fn delete_photo(State(st): State<Shared>, Path((id, view)): Path<(String, String)>) -> ApiResult {
    if !st.store.photos().delete(&id, &view).map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))? {
        return Err(ApiError::not_found("photo", &format!("{id}/{view}")));
    }
    changed(&st, "layouts");
    ok(json!({ "deleted": view }))
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
    changed(&st, "integrations");
    ok(json!({ "reloaded": true }))
}

// ---------------------------------------------------------------- game integrations

fn integration_summary(st: &AppState, s: &crate::integrations::StoredIntegration, detail: bool) -> Value {
    let used_by: Vec<String> = st
        .store
        .profiles()
        .iter()
        .filter_map(|p| p.profile.as_ref())
        .filter(|p| p.outputs.values().any(|o| o.integration.as_deref() == Some(s.id.as_str())))
        .map(|p| p.id.clone())
        .collect();
    let driving = st.snapshot().active.as_ref().is_some_and(|a| a.compiled.outputs.iter().any(|o| o.integration.as_ref().is_some_and(|i| i.id == s.id)));
    let mut v = json!({
        "id": s.id,
        "name": s.def.as_ref().map(|d| d.name.clone()),
        "game": s.def.as_ref().and_then(|d| d.game.as_ref().map(|g| g.name.clone())),
        "valid": s.def.is_some(),
        "errors": s.errors,
        "install": st.store.integrations.install_state(s),
        "driving": driving,
        "used_by": used_by,
    });
    if let Some(d) = &s.def {
        v["live"] = crate::integrations::live(d);
        v["has_launch"] = json!(d.launch.is_some());
        v["source"] = serde_json::to_value(&d.source).unwrap_or(Value::Null);
        v["description"] = json!(d.description);
        v["controls"] = d
            .controls
            .iter()
            .map(|(k, c)| json!({ "id": k, "label": c.label, "kind": c.kind, "group": c.group, "description": c.description }))
            .collect::<Vec<_>>()
            .into();
        v["settings"] = serde_json::to_value(&d.settings).unwrap_or(Value::Null);
        if detail {
            v["definition"] = serde_json::to_value(&**d).unwrap_or(Value::Null);
            v["dir"] = json!(s.dir);
        }
    }
    v
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn list_integrations(State(st): State<Shared>) -> ApiResult {
    let out = blocking(move || st.store.integrations.list().iter().map(|s| integration_summary(&st, s, false)).collect::<Vec<_>>()).await?;
    ok(out)
}

fn find_integration(st: &AppState, id: &str) -> Result<crate::integrations::StoredIntegration, ApiError> {
    st.store.integrations.get(id).ok_or_else(|| ApiError::not_found("integration", id))
}

async fn get_integration(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let s = find_integration(&st, &id)?;
    ok(blocking(move || integration_summary(&st, &s, true)).await?)
}

fn import_error(e: crate::integrations::ImportError) -> ApiError {
    use crate::integrations::ImportError::*;
    match e {
        Invalid(d) => {
            let summary = d.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("; ");
            ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, format!("integration has problems: {summary}")).with("diagnostics", d)
        }
        Exists(id) => ApiError::new(StatusCode::CONFLICT, format!("integration '{id}' already exists; pass replace=true to overwrite")),
        Bad(m) => ApiError::bad(m),
    }
}

/// Re-run the active profile when it drives a changed integration, so the
/// engine uses the new definition.
async fn refresh_integration_users(st: &AppState, id: &str) -> Result<bool, ApiError> {
    let uses = st.snapshot().active.as_ref().and_then(|a| {
        a.compiled.outputs.iter().any(|o| o.integration.as_ref().is_some_and(|i| i.id == id)).then(|| a.compiled.id.clone())
    });
    match uses {
        Some(pid) => {
            let _ = st.activate(&pid).await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportIntegrationBody {
    /// A folder holding integration.json and a files folder, or an integration.json path.
    path: String,
    #[serde(default)]
    replace: bool,
}

async fn import_integration(State(st): State<Shared>, Json(b): Json<ImportIntegrationBody>) -> ApiResult {
    let def = st.store.integrations.import(std::path::Path::new(&b.path), b.replace).map_err(import_error)?;
    changed(&st, "integrations");
    changed(&st, "profiles");
    let reloaded = refresh_integration_users(&st, &def.id).await?;
    let s = find_integration(&st, &def.id)?;
    let st2 = st.clone();
    let mut v = blocking(move || integration_summary(&st2, &s, false)).await?;
    v["imported"] = json!(def.id);
    v["reloaded"] = json!(reloaded);
    ok(v)
}

async fn put_integration(State(st): State<Shared>, Path(id): Path<String>, Json(v): Json<Value>) -> ApiResult {
    let def: vbind_core::integration::Integration = serde_json::from_value(v).map_err(|e| {
        import_error(crate::integrations::ImportError::Invalid(vec![Diagnostic { path: "(file)".into(), message: e.to_string(), pos: None }]))
    })?;
    if def.id != id {
        return Err(ApiError::bad(format!("integration id '{}' does not match the URL '{id}'", def.id)));
    }
    if st.store.integrations.get(&id).is_none() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, format!("no integration '{id}'; import its folder first so its files come with it")));
    }
    st.store.integrations.save_def(&def).map_err(import_error)?;
    changed(&st, "integrations");
    changed(&st, "profiles");
    let reloaded = refresh_integration_users(&st, &id).await?;
    ok(json!({ "saved": id, "reloaded": reloaded }))
}

async fn delete_integration(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    if st.snapshot().active.as_ref().is_some_and(|a| a.compiled.outputs.iter().any(|o| o.integration.as_ref().is_some_and(|i| i.id == id))) {
        return Err(ApiError::new(StatusCode::CONFLICT, format!("the running profile drives '{id}'; stop it first")));
    }
    let removed = st.store.integrations.delete(&id).map_err(|e| ApiError::new(StatusCode::CONFLICT, e))?;
    if !removed {
        return Err(ApiError::not_found("integration", &id));
    }
    changed(&st, "integrations");
    changed(&st, "profiles");
    ok(json!({ "deleted": id }))
}

type IntegrationJob = fn(&crate::integrations::Integrations, &str) -> Result<Value, String>;

async fn integration_job(st: Shared, id: String, job: IntegrationJob) -> ApiResult {
    find_integration(&st, &id)?;
    let st2 = st.clone();
    let r = blocking(move || job(&st2.store.integrations, &id)).await?;
    changed(&st, "integrations");
    r.map(Json).map_err(|e| ApiError::new(StatusCode::CONFLICT, e))
}

async fn install_integration(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    integration_job(st, id, |i, id| i.install(id)).await
}

async fn uninstall_integration(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    integration_job(st, id, |i, id| i.uninstall(id)).await
}

async fn launch_integration(State(st): State<Shared>, Path(id): Path<String>) -> ApiResult {
    integration_job(st, id, |i, id| i.launch(id)).await
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
