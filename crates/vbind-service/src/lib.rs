//! The VBind runtime. [`start`] launches the device engine and the local API
//! on a background thread; the desktop app and `vbind serve` both use it.

pub mod api;
pub mod engine;
pub mod import_hosas;
pub mod integrations;
pub mod panel;
pub mod photos;
pub mod render;
pub mod store;

use std::net::TcpListener as StdListener;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, oneshot};

pub use store::{RuntimeInfo, data_dir, read_runtime};

pub struct Options {
    /// Port to listen on. Default: the saved setting (47821), or any free port.
    pub port: Option<u16>,
}

pub struct ServiceHandle {
    pub port: u16,
    pub token: String,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServiceHandle {
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }

    /// Stop the API, release virtual devices and remove runtime.json.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Block until the service stops (for `vbind serve`).
    pub fn wait(mut self) {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn new_token() -> String {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).expect("system random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Is another VBind answering on the port in runtime.json?
pub fn running_instance() -> Option<RuntimeInfo> {
    let info = read_runtime()?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], info.port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    Some(info)
}

pub fn start(opts: Options) -> Result<ServiceHandle, String> {
    if let Some(other) = running_instance() {
        return Err(format!("VBind is already running (process {}, port {})", other.pid, other.port));
    }
    let store = Arc::new(store::Store::open(data_dir()));
    // A VBind that crashed may have left input in a game's frame.
    store.integrations.clear_frames();
    let want = opts.port.unwrap_or(store.settings().port);
    let listener = StdListener::bind(("127.0.0.1", want))
        .or_else(|_| StdListener::bind(("127.0.0.1", 0)))
        .map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token = new_token();
    let info = RuntimeInfo { port, token: token.clone(), pid: std::process::id(), version: api::VERSION.to_string() };
    std::fs::write(store::runtime_file(), serde_json::to_string_pretty(&info).unwrap())
        .map_err(|e| format!("cannot write {}: {e}", store::runtime_file().display()))?;

    let engine = engine::EngineHandle::start();
    let (changes, _) = broadcast::channel(64);
    let state = Arc::new(api::AppState {
        engine: engine.clone(),
        store,
        panel: Arc::new(panel::Panel::default()),
        token: token.clone(),
        port,
        changes,
        agent: Mutex::new(api::AgentSeen::default()),
        pending: Mutex::new(None),
        started: Instant::now(),
    });
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("vbind-api".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("tokio runtime");
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
                let app = api::router(state.clone());
                let reconcile_state = state.clone();
                let reconciler = tokio::spawn(async move {
                    // Wait for the first device scan, then keep the saved
                    // profile running as devices come and go.
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    let mut last_rev = u64::MAX;
                    let mut last_try = Instant::now() - Duration::from_secs(60);
                    loop {
                        let rev = reconcile_state.engine.shared.devices_rev.load(Ordering::SeqCst);
                        if rev != last_rev || last_try.elapsed() > Duration::from_secs(5) {
                            if rev != last_rev {
                                let _ = reconcile_state.changes.send("devices".into());
                            }
                            last_rev = rev;
                            last_try = Instant::now();
                            reconcile_state.reconcile().await;
                        }
                        tokio::time::sleep(Duration::from_millis(400)).await;
                    }
                });
                let server = axum::serve(listener, app).with_graceful_shutdown(async {
                    let _ = stop_rx.await;
                });
                let _ = server.await;
                reconciler.abort();
                state.engine.deactivate().await;
                state.engine.send(engine::Command::Shutdown);
            });
            if read_runtime().is_some_and(|r| r.pid == std::process::id()) {
                let _ = std::fs::remove_file(store::runtime_file());
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(ServiceHandle { port, token, stop: Some(stop_tx), thread: Some(thread) })
}
