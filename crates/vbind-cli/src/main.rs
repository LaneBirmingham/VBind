//! `vbind`: drive VBind from the command line or as an MCP server.
//!
//! Every command talks to the running VBind over its local API. Add `--json`
//! for machine-readable output. Run `vbind guide` for the agent guide.

mod client;
mod mcp;

use clap::{Parser, Subcommand};
use client::{ApiError, Client};
use serde_json::{Value, json};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "vbind", version, about = "Map any input device to virtual Xbox controllers and joysticks. Agent-first: run `vbind guide`.")]
struct Cli {
    /// Print raw JSON instead of readable text.
    #[arg(long, global = true)]
    json: bool,
    /// Do not start the VBind app when it is not running.
    #[arg(long, global = true)]
    no_start: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Is VBind running, which drivers work, what is active.
    Status,
    /// Print the agent guide (how VBind, profiles and expressions work).
    Guide,
    /// Print a JSON schema: profile, layout, integration or panel.
    Schema { kind: String },
    /// List connected physical devices.
    Devices,
    /// Live inputs, variables and outputs right now.
    State {
        /// Only this device (GUID or role).
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Print input changes as they happen.
    Watch {
        /// Only this device (GUID or role in the active profile).
        #[arg(long)]
        device: Option<String>,
        /// Stop after this many seconds.
        #[arg(long, default_value_t = 30.0)]
        seconds: f64,
    },
    /// Wait for the person to press a button, move a hat or an axis, and report which.
    Capture {
        /// Shown in the VBind window while waiting.
        #[arg(long)]
        prompt: Option<String>,
        /// button, hat or axis (repeatable). Default: all.
        #[arg(long = "kind")]
        kinds: Vec<String>,
        /// Only listen to this device (GUID or role).
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long, default_value_t = 20.0)]
        timeout: f64,
    },
    /// Pretend a device's controls are moved, to test a profile hands-free.
    Simulate {
        /// Device GUID or role. Omit with --clear.
        device: Option<String>,
        /// axis=value, for example y=-1 (repeatable).
        #[arg(long = "axis")]
        axes: Vec<String>,
        /// Button to hold, for example b1 (repeatable).
        #[arg(long = "button")]
        buttons: Vec<String>,
        /// Hat direction, for example hat1=up (repeatable).
        #[arg(long = "hat")]
        hats: Vec<String>,
        /// Seconds to hold.
        #[arg(long = "for", default_value_t = 3.0)]
        duration: f64,
        #[arg(long)]
        profile: Option<String>,
        /// Stop all simulation.
        #[arg(long)]
        clear: bool,
    },
    /// What games see from virtual Xbox pads (XInput slots 0-3).
    Xinput,
    /// Evaluate an expression against live input.
    Eval {
        expr: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Manage profiles.
    #[command(subcommand)]
    Profile(ProfileCmd),
    /// Pick the device for a profile role on this PC.
    Assign {
        profile: String,
        role: String,
        /// Device GUID (see `vbind devices`). Omit with --clear.
        device: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Manage device layout cards.
    #[command(subcommand)]
    Layout(LayoutCmd),
    /// Manage game integrations (optional per-game add-ons).
    #[command(subcommand)]
    Integration(IntegrationCmd),
    /// Post to the agent panel in the VBind window.
    #[command(subcommand)]
    Panel(PanelCmd),
    /// Ask the person a question in the VBind window and wait for the answer.
    Ask {
        question: String,
        /// An answer button (repeatable).
        #[arg(long = "option")]
        options: Vec<String>,
        /// Allow a typed answer.
        #[arg(long)]
        text: bool,
        #[arg(long, default_value_t = 120.0)]
        timeout: f64,
    },
    /// Import settings from other tools.
    #[command(subcommand)]
    Import(ImportCmd),
    /// Run VBind without a window (for agents and testing).
    Serve {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Run as an MCP server on stdin/stdout.
    Mcp,
    /// Open the VBind window.
    Open,
}

#[derive(Subcommand)]
enum ProfileCmd {
    /// List profiles and whether their devices are present.
    List,
    /// Print a profile's JSON.
    Show { id: String },
    /// Add a profile from a file.
    Import {
        file: String,
        /// Overwrite a profile with the same id.
        #[arg(long)]
        replace: bool,
        /// Start it right away.
        #[arg(long)]
        activate: bool,
    },
    /// Save a profile from a file, creating or replacing it.
    Save {
        file: String,
        #[arg(long)]
        activate: bool,
    },
    /// Write a profile to a file (or stdout).
    Export {
        id: String,
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Check a profile file without saving it.
    Validate { file: String },
    /// Start a profile (creates its virtual devices).
    Activate { id: String },
    /// Stop the running profile.
    Deactivate,
    Delete { id: String },
    /// The wiring the window draws: inputs, variables, outputs.
    Graph { id: String },
    /// Which connected device fills each role.
    Roles { id: String },
    /// Pin a memory hint to a mapping: `vbind profile hint <id> pad.a "Index finger"`.
    Hint {
        id: String,
        key: String,
        /// The hint. Omit with --clear.
        text: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Change one mapping: `vbind profile map <id> pad.a "left.b1" --label Boost`.
    Map {
        id: String,
        key: String,
        /// New expression. Omit with --remove.
        expr: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        remove: bool,
    },
}

#[derive(Subcommand)]
enum LayoutCmd {
    List,
    Show { id: String },
    Import { file: String },
    /// The kit of generic parts views are drawn from.
    Kit,
    /// Create or replace one view of a device from a JSON file.
    View { id: String, file: String },
    /// Remove a view.
    RemoveView { id: String, view: String },
    /// Place controls: `vbind layout place <id> --view side b1=34,130 hat1=74,42`.
    Place {
        id: String,
        #[arg(long)]
        view: Option<String>,
        /// control=x,y pairs, or control=none to take one off the picture.
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Draw a view to a PNG file, with a grid and numbered controls.
    Render {
        id: String,
        #[arg(long)]
        view: Option<String>,
        /// Outline the drawing over the view's photo.
        #[arg(long)]
        photo: bool,
        #[arg(short, long, default_value = "layout.png")]
        output: String,
    },
    /// Attach a photo to a view (kept on this PC), or move it with --fit x,y,w,h.
    Photo {
        id: String,
        view: String,
        file: Option<String>,
        /// x,y,width,height in view units (may be negative).
        #[arg(long, allow_hyphen_values = true)]
        fit: Option<String>,
        #[arg(long)]
        remove: bool,
    },
    Delete { id: String },
}

#[derive(Subcommand)]
enum IntegrationCmd {
    /// Installed integrations, whether their game-side files are in place, and live status.
    List,
    /// Everything about one integration, including its definition and live telemetry.
    Show { id: String },
    /// Add an integration from a folder holding integration.json and a files folder.
    Import {
        path: String,
        /// Replace an integration with the same id.
        #[arg(long)]
        replace: bool,
    },
    /// Replace an integration's definition from a JSON file, keeping its files.
    Save { file: String },
    /// Copy the game-side files into the game folder (changes game files; ask first).
    Install { id: String },
    /// Remove the game-side files and put back anything they replaced.
    Uninstall { id: String },
    /// Start the game with the add-on loaded.
    Launch { id: String },
    /// Remove an integration from VBind (uninstall it first).
    Delete { id: String },
}

#[derive(Subcommand)]
enum PanelCmd {
    /// Show what the panel currently holds.
    Show,
    /// Post markdown or cards.
    Post {
        #[arg(long)]
        title: Option<String>,
        /// Markdown text (\n for new lines).
        #[arg(long)]
        markdown: Option<String>,
        /// Read markdown from a file.
        #[arg(long)]
        file: Option<String>,
        /// A JSON file with a full panel post (see `vbind schema panel`).
        #[arg(long = "json-file")]
        json_file: Option<String>,
        /// Add to the panel instead of replacing it.
        #[arg(long)]
        append: bool,
    },
    /// Post a step list: each item is "text" or "text:done|active|failed".
    Steps {
        items: Vec<String>,
        #[arg(long)]
        title: Option<String>,
        /// Card id; posting again with the same id updates it in place.
        #[arg(long, default_value = "steps")]
        id: String,
    },
    /// Highlight controls: role.control (left.b1) or output keys (pad.a).
    Highlight { refs: Vec<String> },
    Clear,
}

#[derive(Subcommand)]
enum ImportCmd {
    /// Galactic Racer HOSAS Bridge hosas_config.json.
    HosasBridge {
        /// Default: %LOCALAPPDATA%\GalacticRacerHOSAS\hosas_config.json
        path: Option<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        activate: bool,
        /// Print the converted profile without saving it.
        #[arg(long)]
        dry_run: bool,
    },
}

fn read_json_file(path: &str) -> Result<Value, ApiError> {
    let text = std::fs::read_to_string(path).map_err(|e| err(format!("cannot read {path}: {e}")))?;
    serde_json::from_str(&text).map_err(|e| err(format!("{path} is not valid JSON: {e}")))
}

fn err(msg: impl Into<String>) -> ApiError {
    ApiError { status: 0, body: json!({ "error": msg.into() }) }
}

fn enc(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32) })
        .collect()
}

fn print(json_mode: bool, v: &Value, human: impl FnOnce(&Value) -> String) {
    if json_mode {
        println!("{}", serde_json::to_string_pretty(v).unwrap());
    } else {
        let s = human(v);
        if !s.is_empty() {
            println!("{s}");
        }
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), ApiError> {
    let j = cli.json;
    let start = !cli.no_start;
    match cli.cmd {
        Cmd::Serve { port } => {
            let h = vbind_service::start(vbind_service::Options { port }).map_err(err)?;
            eprintln!("VBind engine running at {} (Ctrl+C to stop)", h.url());
            h.wait();
            return Ok(());
        }
        Cmd::Mcp => {
            mcp::serve(start);
            return Ok(());
        }
        Cmd::Guide => {
            println!("{}", vbind_service::api::GUIDE);
            return Ok(());
        }
        Cmd::Schema { kind } => {
            let v = vbind_service::api::schema_json(&kind).ok_or_else(|| err("kinds are profile, layout, integration and panel"))?;
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
            return Ok(());
        }
        Cmd::Profile(ProfileCmd::Validate { file }) if client::runtime().is_none() => {
            // Validation works offline too.
            let v = read_json_file(&file)?;
            let p: vbind_core::Profile = serde_json::from_value(v).map_err(|e| err(format!("{file}: {e}")))?;
            let integrations = vbind_service::integrations::load_defs(&vbind_service::data_dir());
            return match p.compile_with(&integrations) {
                Ok(_) => {
                    println!("{file}: valid");
                    Ok(())
                }
                Err(d) => Err(ApiError { status: 422, body: json!({ "error": format!("{file} has problems"), "diagnostics": d }) }),
            };
        }
        Cmd::Open => {
            let app = client::app_path().ok_or_else(|| err("the VBind app is not installed next to vbind.exe"))?;
            std::process::Command::new(app).spawn().map_err(|e| err(e.to_string()))?;
            return Ok(());
        }
        _ => {}
    }
    let c = client::connect("cli", start)?;
    match cli.cmd {
        Cmd::Status => {
            let v = c.get("/status")?;
            print(j, &v, |v| {
                let mut out = format!("VBind {} on port {}  ({} devices, polling {:.0} Hz)\n", s(v, "version"), v["port"], v["devices"], v["polling_hz"].as_f64().unwrap_or(0.0));
                let x = &v["outputs"]["xbox360"];
                out += &format!("Xbox 360 output: {}\n", if x["available"] == true { "ready (ViGEmBus)".into() } else { format!("unavailable: {}", s(x, "error")) });
                let vj = &v["outputs"]["vjoy"];
                out += &format!("vJoy output:     {}\n", if vj["available"] == true { "ready".into() } else { format!("unavailable: {}", s(vj, "error")) });
                out += &format!("Active profile:  {}", v["active_profile"].as_str().unwrap_or("none"));
                if let Some(p) = v["pending"].as_object() {
                    out += &format!("\nWaiting to start '{}': {}", p["profile"].as_str().unwrap_or(""), p["reason"].as_str().unwrap_or(""));
                }
                out
            });
        }
        Cmd::Devices => {
            let v = c.get("/devices")?;
            print(j, &v, |v| {
                let mut out = String::new();
                for d in v.as_array().into_iter().flatten() {
                    let roles: Vec<String> = d["roles"].as_array().into_iter().flatten().map(|r| format!("{}:{}", s(r, "profile"), s(r, "role"))).collect();
                    out += &format!(
                        "{}  {}:{}  {}\n    axes {}  buttons {}  hats {}{}{}\n",
                        s(d, "instance"),
                        d["vid"].as_str().unwrap_or("----"),
                        d["pid"].as_str().unwrap_or("----"),
                        s(d, "name"),
                        d["axes"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(",")).unwrap_or_default(),
                        d["buttons"],
                        d["hats"],
                        d["layout"].as_str().map(|l| format!("  layout {l}")).unwrap_or_default(),
                        if roles.is_empty() { String::new() } else { format!("  role {}", roles.join(", ")) },
                    );
                }
                if out.is_empty() { "No game controllers connected.".into() } else { out.trim_end().into() }
            });
        }
        Cmd::State { device, profile } => {
            let mut q = Vec::new();
            if let Some(d) = device {
                q.push(format!("device={}", enc(&d)));
            }
            if let Some(p) = profile {
                q.push(format!("profile={}", enc(&p)));
            }
            let v = c.get(&format!("/state{}", if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) }))?;
            print(j, &v, human_state);
        }
        Cmd::Watch { device, seconds } => watch(&c, j, device, seconds)?,
        Cmd::Capture { prompt, kinds, device, profile, timeout } => {
            let body = json!({ "prompt": prompt, "kinds": if kinds.is_empty() { Value::Null } else { json!(kinds) }, "device": device, "profile": profile, "timeout_s": timeout });
            if !j {
                eprintln!("Waiting for input ({timeout:.0}s)...");
            }
            let v = c.post("/capture", &body)?;
            print(j, &v, |v| {
                if v["timed_out"] == true {
                    return "Nothing was pressed before the timeout.".into();
                }
                let cap = &v["captured"];
                let mut out = format!("{} on {} ({})", s(cap, "control"), s(cap, "name"), s(cap, "device"));
                if s(cap, "kind") == "axis" {
                    out += &format!(", moved {}", if cap["value"].as_f64().unwrap_or(0.0) < 0.0 { "negative" } else { "positive" });
                }
                if let Some(l) = v["label"].as_str() {
                    out += &format!("\n  {l}");
                }
                for r in v["roles"].as_array().into_iter().flatten() {
                    out += &format!("\n  in profile {}: {}", s(r, "profile"), s(r, "expr"));
                }
                out
            });
        }
        Cmd::Simulate { device, axes, buttons, hats, duration, profile, clear } => {
            if clear {
                let v = c.send("DELETE", "/simulate", None)?;
                print(j, &v, |_| "Simulation cleared.".into());
                return Ok(());
            }
            let device = device.ok_or_else(|| err("name a device (GUID or role), or pass --clear"))?;
            let mut ax = serde_json::Map::new();
            for a in &axes {
                let (k, val) = a.split_once('=').ok_or_else(|| err(format!("write axes as name=value, not '{a}'")))?;
                ax.insert(k.into(), json!(val.parse::<f64>().map_err(|_| err(format!("'{val}' is not a number")))?));
            }
            let bt: serde_json::Map<String, Value> = buttons.iter().map(|b| (b.clone(), json!(true))).collect();
            let mut ht = serde_json::Map::new();
            for h in &hats {
                let (k, d) = h.split_once('=').ok_or_else(|| err(format!("write hats as hat1=up, not '{h}'")))?;
                ht.insert(k.into(), if d == "center" || d == "none" { Value::Null } else { json!(d) });
            }
            let v = c.post("/simulate", &json!({ "device": device, "profile": profile, "axes": ax, "buttons": bt, "hats": ht, "duration_s": duration }))?;
            print(j, &v, |v| format!("Simulating on {} for {}s.", s(v, "device"), v["for_s"]));
        }
        Cmd::Xinput => {
            let v = c.get("/xinput")?;
            print(j, &v, |v| {
                let pads = v.as_array().cloned().unwrap_or_default();
                if pads.is_empty() {
                    return "No XInput pads connected.".into();
                }
                pads.iter()
                    .map(|p| format!("slot {}: buttons {:#06x}  LT {} RT {}  LX {} LY {}  RX {} RY {}", p["slot"], p["buttons"].as_u64().unwrap_or(0), p["left_trigger"], p["right_trigger"], p["thumb_lx"], p["thumb_ly"], p["thumb_rx"], p["thumb_ry"]))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        Cmd::Eval { expr, profile } => {
            let v = c.post("/eval", &json!({ "expr": expr, "profile": profile }))?;
            print(j, &v, |v| format!("{}", v["value"]));
        }
        Cmd::Profile(p) => profile_cmd(&c, j, p)?,
        Cmd::Assign { profile, role, device, clear } => {
            let dev = if clear { Value::Null } else { json!(device.ok_or_else(|| err("give a device GUID, or --clear"))?) };
            let mut body = serde_json::Map::new();
            body.insert(role, dev);
            let v = c.send("PUT", &format!("/profiles/{}/assignments", enc(&profile)), Some(&Value::Object(body)))?;
            print(j, &v, |v| human_roles(&v["roles"]));
        }
        Cmd::Layout(l) => match l {
            LayoutCmd::List => {
                let v = c.get("/layouts")?;
                print(j, &v, |v| v.as_array().into_iter().flatten().map(|l| format!("{}  {}  ({} controls)", s(l, "id"), s(l, "name"), l["controls"].as_object().map(|m| m.len()).unwrap_or(0))).collect::<Vec<_>>().join("\n"));
            }
            LayoutCmd::Show { id } => println!("{}", serde_json::to_string_pretty(&c.get(&format!("/layouts/{}", enc(&id)))?).unwrap()),
            LayoutCmd::Import { file } => {
                let v = c.post("/layouts", &read_json_file(&file)?)?;
                print(j, &v, |v| format!("Saved layout {}.", s(v, "saved")));
            }
            LayoutCmd::Kit => {
                let v = c.get("/layout-kit")?;
                print(j, &v, |v| v.as_array().into_iter().flatten().map(|k| format!("{:<18} {}", s(k, "kit"), s(k, "description"))).collect::<Vec<_>>().join("\n"));
            }
            LayoutCmd::View { id, file } => {
                let view = read_json_file(&file)?;
                let vid = view.get("id").and_then(Value::as_str).ok_or_else(|| err("the view file needs an id"))?.to_string();
                let v = c.send("PUT", &format!("/layouts/{}/views/{}", enc(&id), enc(&vid)), Some(&view))?;
                print(j, &v, |v| format!("Saved view '{vid}' ({} controls placed on it).", v["controls_on_view"]));
            }
            LayoutCmd::RemoveView { id, view } => {
                let v = c.send("DELETE", &format!("/layouts/{}/views/{}", enc(&id), enc(&view)), None)?;
                print(j, &v, |_| format!("Removed view '{view}'."));
            }
            LayoutCmd::Place { id, view, items } => {
                let mut controls = serde_json::Map::new();
                for it in &items {
                    let (k, xy) = it.split_once('=').ok_or_else(|| err(format!("'{it}' should look like b1=34,130")))?;
                    if xy == "none" {
                        controls.insert(k.into(), Value::Null);
                        continue;
                    }
                    let nums: Vec<f64> = xy.split(',').map(|n| n.trim().parse::<f64>()).collect::<std::result::Result<_, _>>().map_err(|_| err(format!("'{xy}' is not x,y")))?;
                    if nums.len() != 2 {
                        return Err(err(format!("'{xy}' is not x,y")));
                    }
                    controls.insert(k.into(), json!(nums));
                }
                let v = c.post(&format!("/layouts/{}/place", enc(&id)), &json!({ "view": view, "controls": controls }))?;
                print(j, &v, |_| format!("Placed {} control(s).", items.len()));
            }
            LayoutCmd::Render { id, view, photo, output } => {
                let mut q = vec![format!("photo={photo}")];
                if let Some(v) = view {
                    q.push(format!("view={}", enc(&v)));
                }
                let png = c.get_bytes(&format!("/layouts/{}/render?{}", enc(&id), q.join("&")))?;
                std::fs::write(&output, &png).map_err(|e| err(format!("cannot write {output}: {e}")))?;
                print(j, &json!({ "written": output }), |_| format!("Wrote {output}."));
            }
            LayoutCmd::Photo { id, view, file, fit, remove } => {
                let path = format!("/layouts/{}/views/{}/photo", enc(&id), enc(&view));
                let v = if remove {
                    c.send("DELETE", &path, None)?
                } else {
                    let mut body = json!({});
                    if let Some(f) = file {
                        let abs = std::fs::canonicalize(&f).map_err(|e| err(format!("cannot find {f}: {e}")))?;
                        body["path"] = json!(abs.to_string_lossy().trim_start_matches(r"\\?\"));
                    }
                    if let Some(f) = fit {
                        let nums: Vec<f64> = f.split(',').map(|n| n.trim().parse::<f64>()).collect::<std::result::Result<_, _>>().map_err(|_| err("--fit is x,y,width,height"))?;
                        body["fit"] = json!(nums);
                    }
                    if body.as_object().is_some_and(|o| o.is_empty()) {
                        return Err(err("give a photo file, --fit or --remove"));
                    }
                    c.send("PUT", &path, Some(&body))?
                };
                print(j, &v, |_| if remove { format!("Removed the photo from '{view}'.") } else { format!("Saved the photo for '{view}'. It stays on this PC.") });
            }
            LayoutCmd::Delete { id } => {
                let v = c.send("DELETE", &format!("/layouts/{}", enc(&id)), None)?;
                print(j, &v, |_| format!("Deleted layout {id}."));
            }
        },
        Cmd::Integration(i) => integration_cmd(&c, j, i)?,
        Cmd::Panel(p) => panel_cmd(&c, j, p)?,
        Cmd::Ask { question, options, text, timeout } => {
            if !j {
                eprintln!("Waiting for an answer in the VBind window...");
            }
            let v = c.post("/panel/ask", &json!({ "question": question, "options": options, "allow_text": text, "timeout_s": timeout }))?;
            print(j, &v, |v| v["answer"].as_str().map(str::to_string).unwrap_or_else(|| "(no answer before the timeout)".into()));
        }
        Cmd::Import(ImportCmd::HosasBridge { path, id, name, replace, activate, dry_run }) => {
            let v = c.post("/import/hosas-bridge", &json!({ "path": path, "id": id, "name": name, "replace": replace, "activate": activate, "save": !dry_run }))?;
            print(j, &v, |v| {
                let mut out = if dry_run { serde_json::to_string_pretty(&v["profile"]).unwrap() + "\n" } else { format!("Imported '{}' from {}.\n", s(v, "saved"), s(v, "source")) };
                for (role, g) in v["assignments"].as_object().into_iter().flatten() {
                    out += &format!("  {role} = {}\n", g.as_str().unwrap_or(""));
                }
                for w in v["warnings"].as_array().into_iter().flatten() {
                    out += &format!("  warning: {}\n", w.as_str().unwrap_or(""));
                }
                if v.get("active").is_some() {
                    out += "Active now.";
                }
                out.trim_end().into()
            });
        }
        Cmd::Serve { .. } | Cmd::Mcp | Cmd::Guide | Cmd::Schema { .. } | Cmd::Open => unreachable!(),
    }
    Ok(())
}

fn human_integration(v: &Value) -> String {
    let mut out = format!("{}  {}", s(v, "id"), v["name"].as_str().unwrap_or("(invalid)"));
    if v["valid"] != true {
        for e in v["errors"].as_array().into_iter().flatten() {
            out += &format!("\n    problem: {}: {}", s(e, "path"), s(e, "message"));
        }
        return out;
    }
    let ins = &v["install"];
    let game = if ins["game_found"] == true {
        let g = &ins["game"];
        let mut t = format!("found at {} (build {})", s(g, "dir"), g["build_id"].as_str().unwrap_or("?"));
        if g["updating"] == true {
            t += ", Steam is updating it";
        }
        match ins["tested_build"].as_bool() {
            Some(true) => t += ", a tested build",
            Some(false) => t += ", NOT a build the add-on was tested on",
            None => {}
        }
        t
    } else {
        "not found through Steam".into()
    };
    out += &format!("\n    game: {game}");
    if ins["needs_install"] == true {
        let state = match (ins["installed"].as_bool(), ins["current"].as_bool()) {
            (Some(true), Some(true)) => "installed".to_string(),
            (Some(true), _) => "installed, but the game folder no longer matches (game update?); install again".to_string(),
            _ => "not installed".to_string(),
        };
        out += &format!("\n    add-on files: {state}");
    }
    let st = &v["live"]["status"];
    if !st.is_null() {
        let state = match s(st, "state") {
            "active" => "active in the game".to_string(),
            "not_running" => "not running (start the game)".to_string(),
            "stale" => "not running (the game closed)".to_string(),
            _ => format!("reports '{}'", s(st, "reported")),
        };
        out += &format!("\n    add-on: {state}");
        for m in st["messages"].as_array().into_iter().flatten() {
            out += &format!("\n      {}", m.as_str().unwrap_or(""));
        }
    }
    for r in v["live"]["readouts"].as_array().into_iter().flatten() {
        if !r["value"].is_null() {
            out += &format!("\n    {}: {}", s(r, "label"), r["value"].as_str().map(str::to_string).unwrap_or_else(|| r["value"].to_string()));
        }
    }
    if v["driving"] == true {
        out += "\n    driven by the active profile";
    }
    out
}

fn integration_cmd(c: &Client, j: bool, cmd: IntegrationCmd) -> Result<(), ApiError> {
    match cmd {
        IntegrationCmd::List => {
            let v = c.get("/integrations")?;
            print(j, &v, |v| {
                let list: Vec<String> = v.as_array().into_iter().flatten().map(human_integration).collect();
                if list.is_empty() { "No integrations installed.".into() } else { list.join("\n") }
            });
        }
        IntegrationCmd::Show { id } => {
            let v = c.get(&format!("/integrations/{}", enc(&id)))?;
            print(j, &v, human_integration);
        }
        IntegrationCmd::Import { path, replace } => {
            let abs = std::fs::canonicalize(&path).map_err(|e| err(format!("cannot find {path}: {e}")))?;
            let v = c.post("/integrations", &json!({ "path": abs.to_string_lossy().trim_start_matches(r"\\?\"), "replace": replace }))?;
            print(j, &v, |v| format!("Imported {}.\n{}", s(v, "imported"), human_integration(v)));
        }
        IntegrationCmd::Save { file } => {
            let def = read_json_file(&file)?;
            let id = def.get("id").and_then(Value::as_str).ok_or_else(|| err("the file needs an id"))?.to_string();
            let v = c.send("PUT", &format!("/integrations/{}", enc(&id)), Some(&def))?;
            print(j, &v, |v| format!("Saved {id}.{}", if v["reloaded"] == true { " The active profile restarted with it." } else { "" }));
        }
        IntegrationCmd::Install { id } => {
            let v = c.post(&format!("/integrations/{}/install", enc(&id)), &json!({}))?;
            print(j, &v, |v| {
                let files: Vec<String> = v["files"].as_array().into_iter().flatten().map(|f| format!("  {}", f.as_str().unwrap_or(""))).collect();
                format!("Installed {id} into {}:\n{}", s(v, "game_dir"), files.join("\n"))
            });
        }
        IntegrationCmd::Uninstall { id } => {
            let v = c.post(&format!("/integrations/{}/uninstall", enc(&id)), &json!({}))?;
            print(j, &v, |v| format!("Uninstalled {id} ({} files removed).", v["files"].as_array().map_or(0, |a| a.len())));
        }
        IntegrationCmd::Launch { id } => {
            let v = c.post(&format!("/integrations/{}/launch", enc(&id)), &json!({}))?;
            print(j, &v, |v| format!("Asked Steam to start the game with: {}", v["args"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")));
        }
        IntegrationCmd::Delete { id } => {
            let v = c.send("DELETE", &format!("/integrations/{}", enc(&id)), None)?;
            print(j, &v, |_| format!("Deleted {id}."));
        }
    }
    Ok(())
}

fn profile_cmd(c: &Client, j: bool, cmd: ProfileCmd) -> Result<(), ApiError> {
    match cmd {
        ProfileCmd::List => {
            let v = c.get("/profiles")?;
            print(j, &v, |v| {
                let list = v.as_array().cloned().unwrap_or_default();
                if list.is_empty() {
                    return "No profiles yet. Import one with `vbind profile import <file>` or have your agent write one.".into();
                }
                list.iter()
                    .map(|p| {
                        let state = if p["active"] == true {
                            "active".to_string()
                        } else if p["valid"] != true {
                            "has errors".into()
                        } else if let Some(r) = p["pending"].as_str() {
                            format!("waiting: {r}")
                        } else {
                            String::new()
                        };
                        format!("{:<32} {:<40} {}", s(p, "id"), s(p, "name"), state)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        ProfileCmd::Show { id } => println!("{}", serde_json::to_string_pretty(&c.get(&format!("/profiles/{}", enc(&id)))?).unwrap()),
        ProfileCmd::Import { file, replace, activate } => {
            let v = c.post(&format!("/profiles?replace={replace}&activate={activate}"), &read_json_file(&file)?)?;
            print(j, &v, |v| if v.get("active").is_some() { format!("Imported and started '{}'.", s(v, "active")) } else { format!("Imported '{}'.\n{}", s(v, "saved"), human_roles(&v["roles"])) });
        }
        ProfileCmd::Save { file, activate } => {
            let body = read_json_file(&file)?;
            let id = body.get("id").and_then(Value::as_str).ok_or_else(|| err("the profile has no id"))?.to_string();
            let v = c.post(&format!("/profiles?replace=true&activate={activate}"), &body)?;
            print(j, &v, |v| format!("Saved '{id}'.{}", if v["reloaded"] == true || v.get("active").is_some() { " Running now." } else { "" }));
        }
        ProfileCmd::Export { id, output } => {
            let v = c.get(&format!("/profiles/{}", enc(&id)))?;
            let text = serde_json::to_string_pretty(&v).unwrap() + "\n";
            match output {
                Some(path) => {
                    std::fs::write(&path, text).map_err(|e| err(format!("cannot write {path}: {e}")))?;
                    eprintln!("Wrote {path}");
                }
                None => print!("{text}"),
            }
        }
        ProfileCmd::Validate { file } => {
            let v = c.post("/profiles/validate", &read_json_file(&file)?)?;
            if j {
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
            } else if v["valid"] == true {
                println!("{file}: valid\n{}", human_roles(&v["roles"]));
            } else {
                return Err(ApiError { status: 422, body: json!({ "error": format!("{file} has problems"), "diagnostics": v["diagnostics"] }) });
            }
        }
        ProfileCmd::Activate { id } => {
            let v = c.post(&format!("/profiles/{}/activate", enc(&id)), &json!({}))?;
            print(j, &v, |v| format!("Started '{}'.\n{}", s(v, "active"), human_roles(&v["roles"])));
        }
        ProfileCmd::Deactivate => {
            let v = c.post("/deactivate", &json!({}))?;
            print(j, &v, |_| "Stopped. Virtual devices unplugged.".into());
        }
        ProfileCmd::Delete { id } => {
            let v = c.send("DELETE", &format!("/profiles/{}", enc(&id)), None)?;
            print(j, &v, |_| format!("Deleted '{id}'."));
        }
        ProfileCmd::Graph { id } => {
            let v = c.get(&format!("/profiles/{}/graph", enc(&id)))?;
            print(j, &v, |v| human_graph(&v["graph"]));
        }
        ProfileCmd::Roles { id } => {
            let v = c.get(&format!("/profiles/{}/roles", enc(&id)))?;
            print(j, &v, human_roles);
        }
        ProfileCmd::Hint { id, key, text, clear } => {
            if text.is_none() && !clear {
                return Err(err("give the hint text, or --clear"));
            }
            let mut body = serde_json::Map::new();
            body.insert(key.clone(), if clear { Value::Null } else { json!(text) });
            let v = c.send("PUT", &format!("/profiles/{}/hints", enc(&id)), Some(&Value::Object(body)))?;
            print(j, &v, |_| if clear { format!("Cleared the hint for {key}.") } else { format!("Saved the hint for {key}.") });
        }
        ProfileCmd::Map { id, key, expr, label, remove } => {
            if expr.is_none() && !remove && label.is_none() {
                return Err(err("give an expression, --label or --remove"));
            }
            let mut body = json!({ "key": key, "label": label });
            if !remove {
                let current = c.get(&format!("/profiles/{}", enc(&id)))?;
                let e = expr.or_else(|| current["map"][&key].as_str().map(str::to_string)).ok_or_else(|| err(format!("'{key}' is not mapped yet; give an expression")))?;
                body["expr"] = json!(e);
            }
            let v = c.post(&format!("/profiles/{}/map", enc(&id)), &body)?;
            print(j, &v, |v| format!("Saved '{}'.{}", s(v, "saved"), if v["reloaded"] == true { " Running now." } else { "" }));
        }
    }
    Ok(())
}

fn panel_cmd(c: &Client, j: bool, cmd: PanelCmd) -> Result<(), ApiError> {
    let v = match cmd {
        PanelCmd::Show => c.get("/panel")?,
        PanelCmd::Post { title, markdown, file, json_file, append } => {
            let mut body = match json_file {
                Some(f) => read_json_file(&f)?,
                None => json!({}),
            };
            let md = match (markdown, file) {
                (Some(m), _) => Some(m.replace("\\n", "\n")),
                (None, Some(f)) => Some(std::fs::read_to_string(&f).map_err(|e| err(format!("cannot read {f}: {e}")))?),
                _ => None,
            };
            if let Some(m) = md {
                body["markdown"] = json!(m);
            }
            if let Some(t) = title {
                body["title"] = json!(t);
            }
            if append {
                body["append"] = json!(true);
            }
            c.post("/panel", &body)?
        }
        PanelCmd::Steps { items, title, id } => {
            let steps: Vec<Value> = items
                .iter()
                .map(|it| match it.rsplit_once(':') {
                    Some((text, st)) if ["done", "active", "todo", "failed"].contains(&st) => json!({ "text": text, "status": st }),
                    _ => json!({ "text": it, "status": "todo" }),
                })
                .collect();
            c.post("/panel", &json!({ "cards": [{ "type": "steps", "id": id, "title": title, "steps": steps }], "append": true }))?
        }
        PanelCmd::Highlight { refs } => {
            let hl: Vec<Value> = refs
                .iter()
                .map(|r| match r.split_once('.') {
                    Some((head, rest)) if !rest.is_empty() => json!({ "role": head, "control": rest }),
                    _ => json!({ "control": r }),
                })
                .collect();
            c.post("/panel", &json!({ "highlights": hl, "append": true }))?
        }
        PanelCmd::Clear => c.send("DELETE", "/panel", None)?,
    };
    print(j, &v, |v| format!("Panel has {} card(s).", v["cards"].as_array().map(|a| a.len()).unwrap_or(0)));
    Ok(())
}

fn human_roles(v: &Value) -> String {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            let dev = r["device"].as_str().unwrap_or("");
            let mut line = format!("  {:<10} {:<10} {dev}", s(r, "role"), s(r, "status"));
            if let Some(c) = r["candidates"].as_array().filter(|c| !c.is_empty()) {
                line += &format!(" candidates: {}", c.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
            }
            if let Some(n) = r["note"].as_str() {
                line += &format!(" ({n})");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn human_graph(g: &Value) -> String {
    let mut out = String::new();
    for r in g["roles"].as_array().into_iter().flatten() {
        out += &format!("{} ({})\n", s(r, "role"), s(r, "label"));
        for c in r["controls"].as_array().into_iter().flatten() {
            out += &format!("    {:<12} {}\n", s(c, "control"), s(c, "label"));
        }
    }
    for v in g["vars"].as_array().into_iter().flatten() {
        out += &format!("var {} = {}{}\n", s(v, "name"), s(v, "expr"), v["label"].as_str().map(|l| format!("   ({l})")).unwrap_or_default());
    }
    for o in g["outputs"].as_array().into_iter().flatten() {
        out += &format!("{} ({})\n", s(o, "name"), s(o, "label"));
        for c in o["controls"].as_array().into_iter().flatten() {
            out += &format!("    {:<12} = {:<36} {}\n", s(c, "control"), s(c, "expr"), c["action"].as_str().unwrap_or(""));
        }
    }
    out.trim_end().into()
}

fn human_state(v: &Value) -> String {
    let mut out = String::new();
    for (g, d) in v["devices"].as_object().into_iter().flatten() {
        let axes: Vec<String> = d["axes"]
            .as_array()
            .into_iter()
            .flatten()
            .zip(vbind_core::state::AXIS_NAMES)
            .filter(|(a, _)| a.as_f64().unwrap_or(0.0).abs() > 0.02)
            .map(|(a, n)| format!("{n}={}", a))
            .collect();
        let hats: Vec<String> = d["hats"].as_array().into_iter().flatten().enumerate().filter(|(_, h)| !h.is_null()).map(|(i, h)| format!("hat{}={}", i + 1, h)).collect();
        let buttons: Vec<String> = d["buttons"].as_array().into_iter().flatten().map(|b| format!("b{b}")).collect();
        out += &format!("{g}: {} {} {}\n", axes.join(" "), hats.join(" "), buttons.join(" "));
    }
    if let Some(a) = v["active"].as_object() {
        out += &format!("profile {}\n", a["profile"].as_str().unwrap_or(""));
        for (k, x) in a["vars"].as_object().into_iter().flatten() {
            out += &format!("  var {k} = {x}\n");
        }
        for (k, x) in a["values"].as_object().into_iter().flatten() {
            if x.as_f64().unwrap_or(0.0) != 0.0 {
                out += &format!("  {k} = {x}\n");
            }
        }
    } else {
        out += "no active profile\n";
    }
    out.trim_end().into()
}

fn watch(c: &Client, j: bool, device: Option<String>, seconds: f64) -> Result<(), ApiError> {
    let filter = match &device {
        Some(d) if d.parse::<vbind_core::state::Guid>().is_ok() => Some(d.to_uppercase()),
        Some(d) => {
            let st = c.get("/state")?;
            Some(st["active"]["roles"][d].as_str().ok_or_else(|| err(format!("role '{d}' is not in the active profile")))?.to_string())
        }
        None => None,
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    let mut last: Option<Value> = None;
    let mut printed: std::collections::HashMap<String, Vec<f64>> = std::collections::HashMap::new();
    if !j {
        eprintln!("Watching input for {seconds:.0}s...");
    }
    c.events(30, |event, data| {
        if std::time::Instant::now() > deadline {
            return false;
        }
        if event != "state" {
            return true;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { return true };
        let devices = v["devices"].clone();
        if j {
            println!("{data}");
            return true;
        }
        if let Some(prev) = &last {
            for (g, d) in devices.as_object().into_iter().flatten() {
                if filter.as_ref().is_some_and(|f| f != g) {
                    continue;
                }
                let p = &prev[g];
                let pb: Vec<&Value> = p["buttons"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
                for b in d["buttons"].as_array().into_iter().flatten() {
                    if !pb.contains(&b) {
                        println!("{g}  b{b} pressed");
                    }
                }
                for (i, h) in d["hats"].as_array().into_iter().flatten().enumerate() {
                    if *h != p["hats"][i] {
                        println!("{g}  hat{} {}", i + 1, if h.is_null() { "centered".into() } else { h.to_string() });
                    }
                }
                let seen = printed.entry(g.clone()).or_insert_with(|| p["axes"].as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(0.0)).collect());
                for (i, a) in d["axes"].as_array().into_iter().flatten().enumerate() {
                    let now = a.as_f64().unwrap_or(0.0);
                    if let Some(before) = seen.get_mut(i) {
                        if (now - *before).abs() >= 0.1 {
                            println!("{g}  {} {:.2}", vbind_core::state::AXIS_NAMES[i], now);
                            *before = now;
                        }
                    }
                }
            }
        }
        last = Some(devices);
        true
    })
}
