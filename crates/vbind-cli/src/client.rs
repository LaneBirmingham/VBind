//! HTTP client for a running VBind, found through runtime.json.

use serde_json::Value;
use std::io::BufRead;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vbind_service::{RuntimeInfo, read_runtime, running_instance};

pub struct Client {
    agent: ureq::Agent,
    base: String,
    token: String,
    client_name: String,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub body: Value,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = self.body.get("error").and_then(Value::as_str).unwrap_or("request failed");
        write!(f, "{msg}")?;
        if let Some(d) = self.body.get("diagnostics").and_then(Value::as_array) {
            for item in d {
                let path = item.get("path").and_then(Value::as_str).unwrap_or("");
                let m = item.get("message").and_then(Value::as_str).unwrap_or("");
                match item.get("pos").and_then(Value::as_u64) {
                    Some(p) => write!(f, "\n  {path}: {m} (at character {})", p + 1)?,
                    None => write!(f, "\n  {path}: {m}")?,
                }
            }
        }
        Ok(())
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;

fn local_error(msg: impl Into<String>) -> ApiError {
    ApiError { status: 0, body: serde_json::json!({ "error": msg.into() }) }
}

/// The desktop app, when installed next to this `vbind.exe`.
pub fn app_path() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    // Not "VBind.exe": Windows names are case-insensitive, so that would find this vbind.exe.
    Some(dir.join("vbind-app.exe")).filter(|p| p.is_file())
}

/// Start the app fully detached. It must not inherit any handle from us:
/// Windows hands every inheritable handle to a child, so an MCP client's
/// stdio pipes would stay open inside the app and the client would never see
/// them close. It also leaves the caller's job when allowed, so it outlives
/// the agent session.
fn launch_detached(app: &std::path::Path) -> std::io::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS,
        PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::core::{PCWSTR, PWSTR};
    let wide = |s: &std::ffi::OsStr| -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain(std::iter::once(0)).collect()
    };
    let exe = wide(app.as_os_str());
    let dir = wide(app.parent().unwrap_or(std::path::Path::new(".")).as_os_str());
    let mut cmdline = wide(std::ffi::OsString::from(format!("\"{}\"", app.display())).as_os_str());
    let si = STARTUPINFOW { cb: std::mem::size_of::<STARTUPINFOW>() as u32, ..Default::default() };
    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT;
    for flags in [base | CREATE_BREAKAWAY_FROM_JOB, base] {
        let mut pi = PROCESS_INFORMATION::default();
        let ok = unsafe {
            CreateProcessW(
                PCWSTR(exe.as_ptr()),
                Some(PWSTR(cmdline.as_mut_ptr())),
                None,
                None,
                false,
                flags,
                None,
                PCWSTR(dir.as_ptr()),
                &si,
                &mut pi,
            )
        };
        if ok.is_ok() {
            unsafe {
                let _ = CloseHandle(pi.hThread);
                let _ = CloseHandle(pi.hProcess);
            }
            return Ok(());
        }
    }
    Err(std::io::Error::last_os_error())
}

/// Find the running VBind, starting the app if it is installed and `start` is set.
pub fn connect(client_name: &str, start: bool) -> Result<Client> {
    let info = match running_instance() {
        Some(i) => i,
        None => {
            let Some(app) = app_path().filter(|_| start) else {
                return Err(local_error(
                    "VBind is not running. Open the VBind app, or run `vbind serve` for a headless engine.",
                ));
            };
            launch_detached(&app).map_err(|e| local_error(format!("could not start {}: {e}", app.display())))?;
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                if let Some(i) = running_instance() {
                    break i;
                }
                if Instant::now() > deadline {
                    return Err(local_error("started the VBind app but it did not come up within 15 seconds"));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    };
    Ok(Client::new(&info, client_name))
}

impl Client {
    pub fn new(info: &RuntimeInfo, client_name: &str) -> Client {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_connect(Some(Duration::from_secs(3))).build().into();
        Client { agent, base: format!("http://127.0.0.1:{}/api", info.port), token: info.token.clone(), client_name: client_name.to_string() }
    }

    fn finish(res: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value> {
        let mut res = res.map_err(|e| local_error(format!("could not reach VBind: {e}")))?;
        let status = res.status().as_u16();
        let text = res.body_mut().read_to_string().map_err(|e| local_error(format!("bad response from VBind: {e}")))?;
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        if (200..300).contains(&status) { Ok(body) } else { Err(ApiError { status, body }) }
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        Self::finish(
            self.agent
                .get(format!("{}{path}", self.base))
                .header("X-VBind-Token", &self.token)
                .header("X-VBind-Client", self.client_name.as_str())
                .call(),
        )
    }

    /// GET raw bytes (for pictures). Errors still come back as JSON.
    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let mut res = self
            .agent
            .get(format!("{}{path}", self.base))
            .header("X-VBind-Token", &self.token)
            .header("X-VBind-Client", self.client_name.as_str())
            .call()
            .map_err(|e| local_error(format!("could not reach VBind: {e}")))?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            let text = res.body_mut().read_to_string().unwrap_or_default();
            return Err(ApiError { status, body: serde_json::from_str(&text).unwrap_or(Value::String(text)) });
        }
        res.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| local_error(format!("bad response from VBind: {e}")))
    }

    pub fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let url = format!("{}{path}", self.base);
        let tok = self.token.as_str();
        let name = self.client_name.as_str();
        let res = match method {
            "POST" => {
                let r = self.agent.post(&url).header("X-VBind-Token", tok).header("X-VBind-Client", name);
                match body {
                    Some(b) => r.send_json(b),
                    None => r.send_empty(),
                }
            }
            "PUT" => self.agent.put(&url).header("X-VBind-Token", tok).header("X-VBind-Client", name).send_json(body.cloned().unwrap_or(Value::Null)),
            "DELETE" => self.agent.delete(&url).header("X-VBind-Token", tok).header("X-VBind-Client", name).call(),
            _ => return self.get(path),
        };
        Self::finish(res)
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.send("POST", path, Some(body))
    }

    /// Stream server-sent events, calling `each(event, data)` until it returns false.
    pub fn events(&self, hz: u32, mut each: impl FnMut(&str, &str) -> bool) -> Result<()> {
        let url = format!("{}/events?hz={hz}&token={}", self.base, self.token);
        let mut res = self
            .agent
            .get(&url)
            .header("X-VBind-Client", self.client_name.as_str())
            .call()
            .map_err(|e| local_error(format!("could not reach VBind: {e}")))?;
        let reader = std::io::BufReader::new(res.body_mut().as_reader());
        let mut event = String::from("message");
        for line in reader.lines() {
            let line = line.map_err(|e| local_error(format!("event stream ended: {e}")))?;
            if let Some(e) = line.strip_prefix("event:") {
                event = e.trim().to_string();
            } else if let Some(d) = line.strip_prefix("data:") {
                if !each(&event, d.trim_start()) {
                    return Ok(());
                }
            } else if line.is_empty() {
                event = "message".into();
            }
        }
        Ok(())
    }
}

/// Read runtime.json without contacting VBind.
pub fn runtime() -> Option<RuntimeInfo> {
    read_runtime()
}
