# VBind agent guide

VBind maps physical input devices (flight sticks, throttles, wheels, pedals,
pads) to virtual devices (an Xbox 360 controller, a vJoy flight stick) on
Windows. You, the agent, do the mapping work: find the devices, learn which
control is which, write a profile, test it, and explain it to the person.
The VBind window shows the live mapping and anything you post to its panel.

VBind starts with no profiles. Everything is a JSON file you can read,
write, validate and share.

## Connecting

Use whichever is available. Both talk to the same running VBind.

- **CLI**: `vbind <command>`. Add `--json` for machine-readable output.
  Run `vbind --help` or `vbind <command> --help`.
- **MCP**: `vbind mcp` is a stdio MCP server exposing the same operations as
  tools (`list_devices`, `capture_input`, `save_profile`, ...).
- **HTTP**: `http://127.0.0.1:<port>/api/...` with header
  `X-VBind-Token: <token>`; port and token are in `%APPDATA%\VBind\runtime.json`.

The CLI and MCP server start the VBind app when it is installed next to
`vbind.exe` and not running yet (pass `--no-start` to stop that). For headless
use run `vbind serve`. `vbind status` shows whether VBind is up, and whether
the Xbox (ViGEmBus) and vJoy drivers are available.

## A typical session

1. `vbind status` and `vbind devices` to see what's plugged in.
2. Post a plan to the panel so the person can follow along:
   `vbind panel steps "Find your sticks" "Map thrust and steering" "Map buttons" "Test in game"`
3. Identify devices and controls by asking the person to press them:
   `vbind capture --prompt "Push your LEFT stick forward"`.
   The result names the device, the control (`y`, `b1`, `hat1.up`) and the
   direction of an axis move (negative `y` means forward on most sticks).
4. Ask questions in the window when you need a decision:
   `vbind ask "Which hand do you hold the T.16000M in?" --option Left --option Right`.
5. Write the profile, then `vbind profile validate file.json` until it is clean.
6. `vbind profile import file.json --activate`. The window shows the wiring.
7. Test without touching the hardware: `vbind simulate left --axis y=-1 --button b1 --for 2`
   then `vbind state` or `vbind xinput` to see the virtual outputs.
8. Ask the person to try it, and adjust. Saving a running profile reloads it
   without unplugging the virtual pad, so the game keeps its controller.

If a device has no layout card yet, write one (see Device layouts) so the
window labels its controls properly ("Trigger" instead of "Button 1").

## Profiles

```json
{
  "vbind": 1,
  "id": "my-game-two-sticks",
  "name": "My Game - two sticks",
  "description": "Both sticks forward to accelerate, difference steers.",
  "game": "My Game",
  "devices": {
    "left":  { "label": "Left hand stick",  "match": [{ "vid": "044F", "pid": "B10A" }] },
    "right": { "label": "Right hand stick", "match": [{ "vid": "046D", "pid": "C215" }] }
  },
  "outputs": { "pad": { "type": "xbox360" } },
  "vars": {
    "l": { "expr": "deadzone(-left.y, 0.06)", "label": "Left stick thrust" },
    "r": { "expr": "deadzone(-right.y, 0.06)", "label": "Right stick thrust" },
    "drive": "hosas_drive(l, r)"
  },
  "map": {
    "pad.rt": "max(drive, 0)",
    "pad.lt": "max(-drive, 0)",
    "pad.lx": "clamp(l - r, -1, 1)",
    "pad.a": "left.b1",
    "pad.dpad_up": "left.hat1.up"
  },
  "labels": { "pad.a": "Boost", "pad.rt": "Accelerate" }
}
```

- `devices`: roles the profile needs. `match` lists ways to recognise a
  device (`vid`, `pid` as hex strings, `name` as a case-insensitive part of
  the product name); any entry matching is enough. Mark a role
  `"optional": true` if the profile should run without it. Never put a PC's
  device GUID in a profile: GUIDs only mean something on one PC. If two
  connected devices match a role (two identical sticks), assign one:
  `vbind assign <profile> <role> <device-guid>` (stored only on this PC).
- `outputs`: virtual devices by name. `{"type": "xbox360"}` or
  `{"type": "vjoy", "vjoy_id": 1}`.
- `vars`: named intermediate values. Use them for anything that combines
  inputs; the window draws each variable as a box between inputs and outputs.
- `map`: output control to expression. One expression per output control.
- `labels`: what an output does in the game. The window shows these.
- `polling_hz`: optional, 100-1000, default 500.

Full JSON schema: `vbind schema profile`.

## Expressions

Values are numbers. Axes read `-1..1`, buttons and hat directions read `1`
when pressed and `0` otherwise. Anything `>= 0.5` counts as true.

**Inputs** are `role.control`:

| Control | Meaning |
|---|---|
| `x` `y` `z` `rx` `ry` `rz` `slider1` `slider2` | axes, `-1..1` (`y` is usually negative when pushed forward) |
| `b1` ... `b128` | buttons, numbered as printed on most sticks and in Windows' game controller panel |
| `hat1.up` `hat1.right` `hat1.down` `hat1.left` | hat direction within 23 degrees (diagonals do not count) |
| `hat1.up_right` `hat1.down_right` `hat1.down_left` `hat1.up_left` | hat diagonals |
| `hat1.x` `hat1.y` | hat as `-1/0/1` components (diagonals count for both) |
| `connected` | `1` while the device is plugged in |

`hat` alone means `hat1`.

**Operators**: `+ - * / %`, comparisons `< <= > >= == !=`, logic
`and or not` (or `&& || !`), and `cond ? a : b`. `true` and `false` are 1 and 0.

**Functions**:

| Function | Does |
|---|---|
| `deadzone(v, dz)` | zero within `dz` of center, rescale the rest (`0.06` = 6%) |
| `deadzone1(v, dz)` | the same for one-way `0..1` levers |
| `curve(v, power, weight=0.5, full=1)` | response curve; `power` 1 is linear, above 1 is softer near center; `full` is the deflection that gives 100% |
| `hosas_drive(l, r, preserve=1)` | two-stick thrust: both forward gives the stronger stick, one back scrubs the other in proportion, both back gives their average (brake/reverse). `preserve=0` averages always |
| `lever(v)` | axis `-1..1` to `0..1` (throttles, triggers) |
| `centered(v)` | `0..1` to `-1..1` |
| `clamp(v, lo, hi)` `min(a, b, ...)` `max(a, b, ...)` `abs` `sign` `round` `sqrt` `pow(a, b)` | the usual |
| `scale(v, in_lo, in_hi, out_lo, out_hi)` | linear remap |
| `step(v, t)` | `1` when `v >= t` |
| `between(v, lo, hi)` | `1` when `lo <= v <= hi` |
| `if(c, a, b)` | `a` when `c` is true, else `b` |
| `toggle(x)` | flips between 0 and 1 each time `x` becomes true (latching modes) |

Errors name the exact place and suggest fixes ("unknown function 'hosas_drv'.
Did you mean hosas_drive()?"). Try expressions live with
`vbind eval "max(-left.y, 0)" --profile <id>`.

## Outputs

**xbox360** (`"type": "xbox360"`):

| Control | Range |
|---|---|
| `lx` `ly` `rx` `ry` | sticks, `-1..1` (`ly`/`ry` positive is up) |
| `lt` `rt` | triggers, `0..1` |
| `a` `b` `x` `y` `lb` `rb` `back` `start` `guide` `ls` `rs` | buttons (`ls`/`rs` are stick clicks) |
| `dpad_up` `dpad_down` `dpad_left` `dpad_right` | D-pad |

**vjoy** (`"type": "vjoy"`): axes `x y z rx ry rz slider1 slider2` (`-1..1`),
buttons `b1..b128`, hats `hat1.up/right/down/left` to `hat4.*` (two
neighbouring directions make a diagonal). Use vJoy to merge several sticks
into one device for games that only accept one. It needs the vJoy driver
(https://github.com/BrunnerInnovation/vJoy/releases) with the device
configured to have the axes and buttons you map.

## Common patterns

- Invert an axis: `-left.y`.
- Two sticks as one Xbox pad, podracer style: see the profile above.
- Throttle lever to a trigger: `"pad.rt": "lever(-throttle.y)"`.
- Throttle with a brake zone at the bottom 10%:
  `"pad.lt": "lever(-throttle.y) < 0.1 ? 1 : 0"`.
- One button for two outputs: map both to the same input.
- Combine two sources on one output: `"pad.a": "max(left.b1, right.b5)"`.
- Hat to stick: `"pad.ry": "right.hat1.y"`.
- Shift layer: `"vars": {"shift": "left.b5"}`, then
  `"pad.x": "right.b2 and not shift"`, `"pad.y": "right.b2 and shift"`.
- Latching mode: `"vars": {"walking": "toggle(left.b7)"}`.
- Two sticks as one vJoy stick: `"stick.x": "right.x"`, `"stick.y": "right.y"`,
  `"stick.slider1": "left.y"`, `"stick.b1": "right.b1"`, `"stick.b13": "left.b1"`.

## Device layouts

A layout card names a device's controls and places them on a simple
drawing. It is shared like a profile.

```json
{
  "vbind_layout": 1,
  "id": "thrustmaster-t16000m",
  "name": "Thrustmaster T.16000M FCS",
  "match": [{ "vid": "044F", "pid": "B10A" }],
  "shape": "stick",
  "controls": {
    "y":  { "label": "Stick forward/back", "group": "stick" },
    "b1": { "label": "Trigger", "group": "grip", "pos": [38, 30] },
    "hat1": { "label": "Hat switch", "group": "head", "pos": [52, 10] }
  },
  "hidden": ["z"]
}
```

`shape` is `stick`, `throttle`, `gamepad`, `wheel`, `pedals` or `generic`.
`pos` is `[x, y]` from 0 to 100 on that drawing. Keys are axis names,
`b1..b128` and `hat1..hat4`. Save with `vbind layout import file.json`.
Learn a new device by capturing each control in turn and asking the person
what it is called.

## The panel

Post what you are doing so the person can follow in the window:

- `vbind panel post --markdown "## Mapping your buttons\nPress each one when asked."`
- `vbind panel steps "Find sticks:done" "Map thrust:active" "Map buttons"` (`:done`, `:active`, `:failed`; default todo)
- `vbind panel highlight left.b1 pad.a` to light up controls in the wiring view
- `vbind ask "Use the trigger for Boost?" --option Yes --option "Pick another"`
- `vbind panel clear`

Cards with the same `id` replace each other when appended, so you can update
a step list in place (`vbind panel post --json card.json --append`).

## Importing other tools

`vbind import hosas-bridge [path]` converts a Galactic Racer HOSAS Bridge
`hosas_config.json`, including its exact device assignments.

## Checking your work

- `vbind state` shows live inputs, every variable and every output value.
- `vbind xinput` shows what games receive from virtual Xbox pads.
- `vbind profile graph <id>` shows the wiring the window draws.
- Drivers: if `xbox360` is unavailable, ViGEmBus is missing
  (https://github.com/nefarius/ViGEmBus/releases). If `vjoy` is unavailable,
  install and configure vJoy.
