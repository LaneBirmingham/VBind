//! `vbind mcp`: a Model Context Protocol server over stdin/stdout. Each tool
//! is a thin call to the running VBind's local API.

use crate::client::{self, ApiError, Client};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

struct Tool {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn none() -> Value {
    obj(json!({}), &[])
}

const DEVICE_DESC: &str = "Device GUID from list_devices, or a role name in the profile (default: the active profile).";

fn tools() -> Vec<Tool> {
    vec![
        Tool { name: "vbind_guide", description: "Read this first. How VBind works: profiles, the expression language, outputs, device layouts, the panel, and a step-by-step mapping workflow.", schema: none },
        Tool { name: "status", description: "Whether VBind is running, which output drivers work (ViGEmBus for Xbox 360, vJoy for joysticks), the active profile, and why a saved profile is waiting.", schema: none },
        Tool { name: "list_devices", description: "Connected physical game controllers: GUID, USB vid/pid, name, axes, button and hat counts, layout card, and roles they fill.", schema: none },
        Tool {
            name: "get_state",
            description: "Live values right now: every device's axes (-1..1), hats and pressed buttons, plus the active profile's variables, output values and virtual device reports.",
            schema: || obj(json!({ "device": { "type": "string", "description": DEVICE_DESC }, "profile": { "type": "string" } }), &[]),
        },
        Tool {
            name: "capture_input",
            description: "Ask the person to press a button, move a hat or move an axis, and report exactly which device and control it was (for example b1, hat1.up, or y with its direction). The prompt shows in the VBind window. Returns timed_out if nothing happens.",
            schema: || obj(json!({
                "prompt": { "type": "string", "description": "What to ask, for example 'Pull the trigger on your LEFT stick'." },
                "kinds": { "type": "array", "items": { "enum": ["button", "hat", "axis"] }, "description": "What to listen for. Default: all." },
                "device": { "type": "string", "description": DEVICE_DESC },
                "profile": { "type": "string" },
                "timeout_s": { "type": "number", "description": "Default 20, max 300." }
            }), &[]),
        },
        Tool {
            name: "simulate_input",
            description: "Pretend controls on a connected device are held for a while, to test a profile without touching the hardware. Check the result with get_state or read_xinput.",
            schema: || obj(json!({
                "device": { "type": "string", "description": DEVICE_DESC },
                "profile": { "type": "string" },
                "axes": { "type": "object", "additionalProperties": { "type": "number" }, "description": "Axis name to value, {\"y\": -1}." },
                "buttons": { "type": "object", "additionalProperties": { "type": "boolean" }, "description": "{\"b1\": true}" },
                "hats": { "type": "object", "additionalProperties": { "type": ["string", "null"] }, "description": "{\"hat1\": \"up\"}" },
                "duration_s": { "type": "number", "description": "Default 3." }
            }), &["device"]),
        },
        Tool { name: "clear_simulation", description: "Stop all simulated input.", schema: none },
        Tool { name: "read_xinput", description: "What games receive from virtual Xbox pads: XInput slots 0-3 with buttons, triggers and sticks.", schema: none },
        Tool {
            name: "eval_expression",
            description: "Evaluate one expression against live input using a profile's roles and variables, for example 'max(-left.y, 0)'.",
            schema: || obj(json!({ "expr": { "type": "string" }, "profile": { "type": "string", "description": "Default: the active profile." } }), &["expr"]),
        },
        Tool { name: "list_profiles", description: "All profiles with validity, errors, whether active or waiting, and how each role resolves to a connected device.", schema: none },
        Tool { name: "get_profile", description: "A profile's full JSON.", schema: || obj(json!({ "id": { "type": "string" } }), &["id"]) },
        Tool {
            name: "validate_profile",
            description: "Check a profile without saving. Returns diagnostics with exact paths and positions, role resolution, and the wiring graph when valid.",
            schema: || obj(json!({ "profile": { "type": "object", "description": "A full profile (see get_schema kind=profile)." } }), &["profile"]),
        },
        Tool {
            name: "save_profile",
            description: "Create or replace a profile (validated first). Saving the running profile reloads it without unplugging the virtual pad.",
            schema: || obj(json!({ "profile": { "type": "object" }, "activate": { "type": "boolean", "description": "Start it after saving." } }), &["profile"]),
        },
        Tool { name: "activate_profile", description: "Start a profile: creates its virtual devices and begins mapping. VBind restarts it automatically next time.", schema: || obj(json!({ "id": { "type": "string" } }), &["id"]) },
        Tool { name: "deactivate", description: "Stop mapping and unplug virtual devices.", schema: none },
        Tool { name: "delete_profile", description: "Delete a profile.", schema: || obj(json!({ "id": { "type": "string" } }), &["id"]) },
        Tool { name: "get_profile_graph", description: "The wiring the VBind window draws for a profile: input controls with labels, variables, outputs with game actions, and which inputs feed each output.", schema: || obj(json!({ "id": { "type": "string" } }), &["id"]) },
        Tool {
            name: "assign_device",
            description: "Pick which connected device fills a role on this PC (needed when two identical devices match). Pass device null to clear.",
            schema: || obj(json!({ "profile": { "type": "string" }, "role": { "type": "string" }, "device": { "type": ["string", "null"], "description": "Device GUID" } }), &["profile", "role", "device"]),
        },
        Tool { name: "list_layouts", description: "Device layout cards: names and positions for each device model's controls.", schema: none },
        Tool {
            name: "save_layout",
            description: "Create or replace a device layout card so the window labels and draws a device's controls (see get_schema kind=layout).",
            schema: || obj(json!({ "layout": { "type": "object" } }), &["layout"]),
        },
        Tool {
            name: "post_panel",
            description: "Post to the agent panel in the VBind window: markdown, a step list, cards, and highlighted controls. Replaces the panel unless append is true; appended cards with an existing id update in place.",
            schema: || obj(json!({
                "title": { "type": "string" },
                "markdown": { "type": "string" },
                "steps": { "type": "array", "items": { "type": "object", "properties": { "text": { "type": "string" }, "detail": { "type": "string" }, "status": { "enum": ["done", "active", "todo", "failed"] } }, "required": ["text"] } },
                "cards": { "type": "array", "items": { "type": "object" }, "description": "Cards: {type: markdown|steps|question, id, ...} (see get_schema kind=panel)." },
                "highlights": { "type": "array", "items": { "type": "object", "properties": { "role": { "type": "string" }, "device": { "type": "string" }, "control": { "type": "string" } }, "required": ["control"] }, "description": "Controls to light up: {role: 'left', control: 'b1'} or {control: 'pad.a'}." },
                "append": { "type": "boolean" }
            }), &[]),
        },
        Tool { name: "clear_panel", description: "Remove everything from the agent panel.", schema: none },
        Tool {
            name: "ask_user",
            description: "Show a question with answer buttons (and optionally a text box) in the VBind window and wait for the person's answer.",
            schema: || obj(json!({ "question": { "type": "string" }, "options": { "type": "array", "items": { "type": "string" } }, "allow_text": { "type": "boolean" }, "timeout_s": { "type": "number", "description": "Default 120." } }), &["question"]),
        },
        Tool {
            name: "import_hosas_bridge",
            description: "Convert a Galactic Racer HOSAS Bridge hosas_config.json (default location if no path) into a VBind profile, with the same device assignments. save=false only returns the converted profile.",
            schema: || obj(json!({ "path": { "type": "string" }, "id": { "type": "string" }, "name": { "type": "string" }, "save": { "type": "boolean" }, "replace": { "type": "boolean" }, "activate": { "type": "boolean" } }), &[]),
        },
        Tool { name: "get_schema", description: "JSON schema for a profile, layout or panel post.", schema: || obj(json!({ "kind": { "enum": ["profile", "layout", "panel"] } }), &["kind"]) },
    ]
}

fn enc(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

fn arg<'a>(a: &'a Value, k: &str) -> Result<&'a str, ApiError> {
    a.get(k).and_then(Value::as_str).ok_or_else(|| ApiError { status: 400, body: json!({ "error": format!("missing argument '{k}'") }) })
}

fn call_tool(c: &Client, name: &str, a: &Value) -> Result<Value, ApiError> {
    match name {
        "status" => c.get("/status"),
        "list_devices" => c.get("/devices"),
        "get_state" => {
            let mut q = Vec::new();
            if let Some(d) = a.get("device").and_then(Value::as_str) {
                q.push(format!("device={}", enc(d)));
            }
            if let Some(p) = a.get("profile").and_then(Value::as_str) {
                q.push(format!("profile={}", enc(p)));
            }
            c.get(&format!("/state{}", if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) }))
        }
        "capture_input" => c.post("/capture", a),
        "simulate_input" => c.post("/simulate", a),
        "clear_simulation" => c.send("DELETE", "/simulate", None),
        "read_xinput" => c.get("/xinput"),
        "eval_expression" => c.post("/eval", a),
        "list_profiles" => c.get("/profiles"),
        "get_profile" => c.get(&format!("/profiles/{}", enc(arg(a, "id")?))),
        "validate_profile" => c.post("/profiles/validate", a.get("profile").unwrap_or(&Value::Null)),
        "save_profile" => {
            let p = a.get("profile").cloned().unwrap_or(Value::Null);
            let activate = a.get("activate").and_then(Value::as_bool).unwrap_or(false);
            c.post(&format!("/profiles?replace=true&activate={activate}"), &p)
        }
        "activate_profile" => c.post(&format!("/profiles/{}/activate", enc(arg(a, "id")?)), &json!({})),
        "deactivate" => c.post("/deactivate", &json!({})),
        "delete_profile" => c.send("DELETE", &format!("/profiles/{}", enc(arg(a, "id")?)), None),
        "get_profile_graph" => c.get(&format!("/profiles/{}/graph", enc(arg(a, "id")?))),
        "assign_device" => {
            let mut body = serde_json::Map::new();
            body.insert(arg(a, "role")?.to_string(), a.get("device").cloned().unwrap_or(Value::Null));
            c.send("PUT", &format!("/profiles/{}/assignments", enc(arg(a, "profile")?)), Some(&Value::Object(body)))
        }
        "list_layouts" => c.get("/layouts"),
        "save_layout" => c.post("/layouts", a.get("layout").unwrap_or(&Value::Null)),
        "post_panel" => c.post("/panel", a),
        "clear_panel" => c.send("DELETE", "/panel", None),
        "ask_user" => c.post("/panel/ask", a),
        "import_hosas_bridge" => c.post("/import/hosas-bridge", a),
        "get_schema" => vbind_service::api::schema_json(arg(a, "kind")?).ok_or_else(|| ApiError { status: 400, body: json!({ "error": "kinds are profile, layout and panel" }) }),
        other => Err(ApiError { status: 404, body: json!({ "error": format!("unknown tool '{other}'") }) }),
    }
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Serve MCP on stdin/stdout until stdin closes.
pub fn serve(start_app: bool) {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut client: Option<Client> = None;
    let mut agent_name = String::from("mcp");
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let r = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } });
                let _ = writeln!(stdout, "{r}");
                let _ = stdout.flush();
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => {
                if let Some(n) = params.pointer("/clientInfo/name").and_then(Value::as_str).filter(|n| !n.is_empty()) {
                    agent_name = format!("{} (MCP)", n.chars().take(40).collect::<String>());
                }
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSIONS[0]);
                let version = if PROTOCOL_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSIONS[0] };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false }, "resources": { "listChanged": false } },
                    "serverInfo": { "name": "vbind", "title": "VBind input mapper", "version": vbind_service::api::VERSION },
                    "instructions": "VBind maps physical game controllers to virtual Xbox 360 pads and vJoy sticks on Windows. Call vbind_guide first. Typical flow: list_devices, post_panel with your plan, capture_input to identify controls, validate_profile, save_profile with activate, simulate_input + get_state to test, then ask the person to try it."
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools().iter().map(|t| json!({ "name": t.name, "description": t.description, "inputSchema": (t.schema)() })).collect::<Vec<_>>() })),
            "resources/list" => Ok(json!({ "resources": [{ "uri": "vbind://guide", "name": "VBind agent guide", "mimeType": "text/markdown" }] })),
            "resources/read" => {
                if params.get("uri").and_then(Value::as_str) == Some("vbind://guide") {
                    Ok(json!({ "contents": [{ "uri": "vbind://guide", "mimeType": "text/markdown", "text": vbind_service::api::GUIDE }] }))
                } else {
                    Err((-32602, "unknown resource".into()))
                }
            }
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                if name == "vbind_guide" {
                    Ok(text_result(vbind_service::api::GUIDE.to_string(), false))
                } else if name == "get_schema" {
                    Ok(match call_tool_offline(name, &args) {
                        Ok(v) => text_result(serde_json::to_string_pretty(&v).unwrap(), false),
                        Err(e) => text_result(e.to_string(), true),
                    })
                } else {
                    if client.is_none() {
                        client = client::connect(&agent_name, start_app).ok();
                    }
                    let res = match &client {
                        Some(c) => call_tool(c, name, &args),
                        None => client::connect(&agent_name, start_app).and_then(|c| {
                            let r = call_tool(&c, name, &args);
                            client = Some(c);
                            r
                        }),
                    };
                    // A restarted VBind has a new port and token; reconnect once.
                    let res = match res {
                        Err(e) if e.status == 0 || e.status == 401 => match client::connect(&agent_name, start_app) {
                            Ok(c) => {
                                let r = call_tool(&c, name, &args);
                                client = Some(c);
                                r
                            }
                            Err(e2) => Err(e2),
                        },
                        other => other,
                    };
                    Ok(match res {
                        Ok(v) => text_result(serde_json::to_string_pretty(&v).unwrap(), false),
                        Err(e) => text_result(format!("{e}\n{}", serde_json::to_string_pretty(&e.body).unwrap()), true),
                    })
                }
            }
            m if m.starts_with("notifications/") => continue,
            _ => Err((-32601, format!("method '{method}' not found"))),
        };
        let Some(id) = id else { continue };
        let reply = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        };
        let _ = writeln!(stdout, "{reply}");
        let _ = stdout.flush();
    }
}

fn call_tool_offline(name: &str, a: &Value) -> Result<Value, ApiError> {
    match name {
        "get_schema" => vbind_service::api::schema_json(arg(a, "kind")?).ok_or_else(|| ApiError { status: 400, body: json!({ "error": "kinds are profile, layout and panel" }) }),
        _ => unreachable!(),
    }
}
