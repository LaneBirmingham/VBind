# VBind

VBind maps physical game controllers (flight sticks, throttles, wheels, pedals,
pads) to virtual devices on Windows: a virtual Xbox 360 controller, or a vJoy
flight stick that merges several sticks into one device.

It is agent-first. VBind ships with no mappings. Your AI coding agent connects
to it through MCP or the `vbind` command line, finds your devices, asks you to
press controls, writes the mapping, tests it, and shows its progress in the
VBind window. The window's wiring board shows every mapping live, so you can
see exactly which physical control drives which virtual one.

## Install

Run `VBind_<version>_x64-setup.exe`. It installs per user into
`%LOCALAPPDATA%\VBind`:

- `vbind-app.exe`: the app (Start menu: VBind). Closing the window keeps your
  mapping running from the tray; use Quit in the tray menu to stop it.
- `vbind.exe`: the command line and MCP server your agent uses.

Drivers:

- Virtual Xbox controllers need [ViGEmBus](https://github.com/nefarius/ViGEmBus/releases).
- Virtual flight sticks need [vJoy](https://github.com/BrunnerInnovation/vJoy/releases),
  with device 1 configured in "Configure vJoy".

Your profiles, device choices and layouts live in `%APPDATA%\VBind`.

## Connect your agent

Click **Connect agent** in the app for copy-ready commands, or:

```
claude mcp add --scope user vbind -- "%LOCALAPPDATA%\VBind\vbind.exe" mcp
codex mcp add vbind -- "%LOCALAPPDATA%\VBind\vbind.exe" mcp
```

Other MCP apps: command `%LOCALAPPDATA%\VBind\vbind.exe`, args `["mcp"]`.
Agents without MCP can run `vbind.exe guide` and use the CLI directly.

Then ask something like: "Use VBind to map my two flight sticks to a virtual
Xbox controller for my game. Show me the plan in the VBind window first."

The agent's reference is [docs/AGENT_GUIDE.md](docs/AGENT_GUIDE.md) (also
`vbind guide`, the MCP `vbind_guide` tool, and the `vbind://guide` resource).

## Profiles

A profile is one JSON file: device roles (matched by USB id or name, so it
works on other PCs), optional variables, and one expression per virtual
control, for example:

```json
{
  "vbind": 1,
  "id": "two-sticks",
  "name": "Two sticks",
  "devices": {
    "left": { "label": "Left stick", "match": [{ "vid": "044F", "pid": "B10A" }] },
    "right": { "label": "Right stick", "match": [{ "vid": "046D", "pid": "C215" }] }
  },
  "outputs": { "pad": { "type": "xbox360" } },
  "vars": { "drive": "hosas_drive(deadzone(-left.y, 0.06), deadzone(-right.y, 0.06))" },
  "map": {
    "pad.rt": "max(drive, 0)",
    "pad.lt": "max(-drive, 0)",
    "pad.a": "left.b1"
  },
  "labels": { "pad.a": "Boost" }
}
```

Share a profile with **Export** on the Profiles tab; whoever imports it picks
their own devices for each role. `vbind schema profile` prints the full JSON
schema. Layout cards (`examples/layouts`) name and place a device's controls
so the window can label and draw them.

`examples/profiles/galactic-racer-dual-hosas.vbind.json` is a complete
two-stick HOSAS setup for STAR WARS: Galactic Racer, converted from the
Galactic Racer HOSAS Bridge with `vbind import hosas-bridge`.

## Build from source

Needs Rust 1.90+ (MSVC) and Node 20+.

```
npm install
npm --prefix ui install
npm run app      # UI, CLI, sidecar, then the NSIS installer
```

The installer lands in `target\release\bundle\nsis`. Other scripts:
`npm run ui` (window UI into `ui/dist`, embedded into the engine at build
time), `npm run cli` (`target\release\vbind.exe`), `npm run dev` (run the
app). `vbind serve` runs the engine headless; open the printed URL in a
browser to get the same UI.

Layout:

| Path | What |
| --- | --- |
| `crates/vbind-core` | Profiles, expression language, role matching, layouts, mapping graph |
| `crates/vbind-io` | DirectInput reading, ViGEmBus and vJoy output, XInput readback |
| `crates/vbind-service` | Engine thread (500 Hz), local HTTP API with live events, agent panel |
| `crates/vbind-cli` | `vbind` command line and MCP server |
| `crates/vbind-app` | Tauri 2 desktop app |
| `ui` | Window UI (Preact + Vite) |

`cargo test --workspace` runs the tests, including a check that the Galactic
Racer import reproduces the bridge's math across the whole stick range.
