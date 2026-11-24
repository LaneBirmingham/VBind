# Building and installing

There are no prebuilt releases yet. These steps are written so a coding agent
can follow them end to end on the user's PC.

## Requirements

- Windows 10 or 11, x64.
- [Rust](https://rustup.rs) 1.90+ with the MSVC toolchain
  (`stable-x86_64-pc-windows-msvc`) and the Visual Studio C++ build tools.
- [Node.js](https://nodejs.org) 20+.

## Build the installer

From the repository root:

```
npm install
npm --prefix ui install
npm run app
```

`npm run app` builds the window UI, the `vbind` CLI, stages the CLI as the
app's sidecar, then builds the NSIS installer into
`target\release\bundle\nsis\VBind_<version>_x64-setup.exe`.

## Install

Run the installer. It installs per user (no admin) into `%LOCALAPPDATA%\VBind`:

- `vbind-app.exe` - the app (Start menu: VBind).
- `vbind.exe` - the command line and MCP server agents use.

Do not rename the app to `VBind.exe`: Windows paths are case-insensitive, so
it would collide with `vbind.exe`.

To upgrade, rebuild and run the new installer. Quit VBind from the tray first.

## Drivers

Install the ones the user's profiles need, and ask the user before installing
either (both are kernel drivers and need admin):

- Virtual Xbox 360 controller: [ViGEmBus](https://github.com/nefarius/ViGEmBus/releases).
- Virtual flight stick: [vJoy](https://github.com/BrunnerInnovation/vJoy/releases),
  with device 1 configured in "Configure vJoy" to have the axes and buttons
  the profile maps.

`vbind status` reports whether each driver is available.

## Connect an agent

```
claude mcp add --scope user vbind -- "%LOCALAPPDATA%\VBind\vbind.exe" mcp
codex mcp add vbind -- "%LOCALAPPDATA%\VBind\vbind.exe" mcp
```

Other MCP apps: command `%LOCALAPPDATA%\VBind\vbind.exe`, args `["mcp"]`.
Agents without MCP can run `vbind.exe guide` and use the CLI directly. The
app's **Connect agent** button shows the same commands.

The agent's reference is [AGENT_GUIDE.md](AGENT_GUIDE.md), also available as
`vbind guide`, the MCP `vbind_guide` tool and the `vbind://guide` resource.

## Development

| Command | Does |
| --- | --- |
| `npm run ui` | Build the window UI into `ui/dist` (embedded into the engine at compile time) |
| `npm run cli` | Build `target\release\vbind.exe` |
| `npm run dev` | Build and run the desktop app |
| `vbind serve` | Run the engine headless; open the printed URL in a browser for the same UI |
| `cargo test --workspace` | Run the tests |

The tests include a check that the Galactic Racer import reproduces the HOSAS
Bridge's math across the whole stick range.

For UI work, run the Vite dev server against a running engine:
`VBIND_PORT=<port> npm --prefix ui run dev`, then open the page with
`?token=<token>` from `%APPDATA%\VBind\runtime.json`. To keep the installed
engine untouched, start a second one with its own data folder:
`VBIND_HOME=<scratch dir> vbind serve --port <port>`.

## Project layout

| Path | What |
| --- | --- |
| `crates/vbind-core` | Profiles, expression language, role matching, layouts, mapping graph |
| `crates/vbind-io` | DirectInput reading, ViGEmBus, vJoy and SendInput output, XInput readback |
| `crates/vbind-service` | Engine thread (500 Hz), local HTTP API with live events, agent panel |
| `crates/vbind-cli` | `vbind` command line and MCP server |
| `crates/vbind-app` | Tauri 2 desktop app |
| `ui` | Window UI (Preact + Vite) |
| `examples` | Example profiles and device layouts |
