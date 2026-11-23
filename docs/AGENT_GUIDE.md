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
window labels its controls properly ("Trigger" instead of "Button 1"), then
draw it (see Drawing a device) so the person can see their mapping on a
picture of their own device and practise it in the trainer.

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
- `outputs`: virtual devices by name. `{"type": "xbox360"}`,
  `{"type": "vjoy", "vjoy_id": 1}`, or a game integration
  `{"type": "integration", "integration": "<id>"}` (see Game integrations).
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
| `taps(x, n=2, ms=60)` | each time `x` becomes true, `n` quick taps (`ms` on, `ms` off); for keys a game wants double-tapped |

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

**integration** (`"type": "integration", "integration": "<id>"`): the
controls the integration defines (`vbind integration show <id>`), with
their own kinds. Optional `settings` override the integration's defaults.
A setting can also be mapped like a control, as `<output>.settings.<name>`,
so a button changes it while playing; VBind rewrites the settings channel
whenever the value changes. A setting with `choices` takes the choice at the
expression's value (0 is the first, so `toggle(left.b11)` flips between the
first two), an on/off setting turns on at 0.5, and a number setting takes the
value, clamped to its range. Map a setting or fix it under `settings`, not both.

**keyboard** (`"type": "keyboard"`): keys and the mouse, for games or
actions that only take keyboard and mouse input. Keys are `key.<name>` by
US layout position (`key.a`, `key.1`, `key.f1`, `key.lalt`, `key.lctrl`,
`key.space`, `key.up`, `key.numpad5`) and are held while the expression is
true; mouse buttons are `mouse.left`, `mouse.right`, `mouse.middle`,
`mouse.x1`, `mouse.x2`; `mouse.x`, `mouse.y` (positive is right and down) and
`mouse.wheel` (positive scrolls up) take -1 to 1 as a speed. Optional
`settings`: `mouse_speed` (pixels per second at full deflection, default 1200)
and `wheel_speed` (notches per second, default 8). Input goes to the focused
window, and everything held is released when the profile stops. Example, a
hat that holds Alt and moves the mouse for free look:

```json
"outputs": { "kbm": { "type": "keyboard", "settings": { "mouse_speed": 900 } } },
"map": { "kbm.key.lalt": "right.hat1.x != 0 || right.hat1.y != 0",
         "kbm.mouse.x": "right.hat1.x", "kbm.mouse.y": "-right.hat1.y" }
```

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
- One button that double-taps a key: `"kbm.key.lalt": "taps(right.b6)"`;
  combine with a hold as `right.hat1.x != 0 || taps(right.b6)`.
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
what it is called. A card can also carry real pictures of the device in
`views`, with each control placed by `view` and `at`; see the next section.

## Drawing a device

The Mapping tab has a Picture view: each device drawn as itself, with every
mapped control pointing at the game button it drives, plus a trainer that
shows a game button and waits for the person to press it. It exists to help
people learn which control a game's "Press A" means. VBind ships no device
art, so you draw each device into its layout card. Any device works: sticks,
throttles, button boxes, pedals, wheels.

1. Work out what the device looks like: its name and USB ID from
   `vbind devices`, product photos or the manual if you can browse, and the
   person's description. Pick one to three views, for example the stick from
   the side and the base from above.
2. Confirm which control is which before placing anything. Button numbers
   are not printed on most devices:
   `vbind capture --prompt "Press the button at the top left of the head"`.
3. Draw each view with `set_layout_view` (`vbind layout view <layout> view.json`).
   A view has a `size` in view units (any scale, for example 140 x 300 for a
   stick) and `parts`, drawn in order so later parts sit on top:
   - `{"kit": "grip.contoured", "box": [x, y, w, h]}` places a kit part,
     stretched to fill the box. `rotate` (degrees) and `flip` (mirror) are
     optional. `vbind layout kit` (`layout_kit`) lists the parts: grips,
     heads, bases, boots, triggers, hats, buttons, thumbsticks, wheels,
     sliders, toggles, levers, throttle handles, pedals, a gamepad body, a
     steering wheel rim and plain shapes.
   - `{"path": "M40 92 C34 62 46 30 74 26 ..."}` is SVG path data in view
     units, for shapes the kit has no part for.
   - `style` is `body` (the device), `detail` (raised caps), `hole`
     (recesses) or `line` (seams and outlines).
4. Place controls with `place_controls`
   (`vbind layout place <layout> --view side b1=34,130 hat1=74,42`), at the
   centre of each button or hat. Axes can be placed too, for example `y` on
   the middle of the grip, so thrust and steering get a callout.
5. Look at your work with `render_layout`
   (`vbind layout render <layout> --view side -o side.png`): the drawing on a
   grid in view units, each placed control as a numbered marker, and a legend
   that lists anything not placed yet. Fix and render again until it looks
   like the device and every mapped control sits on its button.

```json
{
  "id": "side",
  "label": "Stick",
  "size": [140, 300],
  "parts": [
    { "kit": "base.rounded", "box": [0, 256, 140, 42] },
    { "kit": "boot", "box": [50, 238, 40, 20] },
    { "kit": "grip.contoured", "box": [42, 88, 60, 160] },
    { "kit": "trigger", "box": [30, 108, 18, 40] },
    { "kit": "head.wide", "box": [33, 24, 83, 72] },
    { "kit": "hat", "box": [64, 32, 20, 20] }
  ]
}
```

Photos: the person can add a photo of their own device to a view in the
window, or you can attach one they point you to with `set_layout_photo`
(`vbind layout photo <layout> <view> photo.jpg`). Line it up by setting `fit`,
`[x, y, width, height]` in view units, and check with `render_layout`
`photo=true`, which outlines your drawing over the photo. Photos stay on this
PC and are never part of a shared layout, so never put product photos in a
layout.

Memory hints: `set_hints` (`vbind profile hint <profile> pad.a "Index finger,
like a gas pedal"`) pins a short, physical reminder to a mapping. It shows in
the callout and in the trainer.


## Game integrations

Some games take no controller input worth using: no gamepad support, or
only digital keys. If someone has written a game-side add-on (a plugin or
mod that runs inside the game and accepts input from outside), VBind can
drive it directly through a game integration, with true analog input. An
integration is optional and per PC. VBind ships none and has nothing
game-specific built in: it is a data file plus the add-on's files, kept in
`%APPDATA%\VBind\integrations\<id>\`.

Check `vbind integration list` (`list_integrations`) first. To add one:

1. Find the add-on and read how it receives input. VBind speaks one
   transport today: a fixed-layout frame in named shared memory, which the
   add-on reads every game frame. Kontrol adapters
   (https://github.com/komandio-labs/kontrol-adapters) work this way, so any
   game they cover can be added.
2. Write `integration.json` (`vbind schema integration`):
   - `frame`: the shared memory name, its size in bytes, and `constants`
     (version numbers, an "input enabled" flag) written while a profile runs.
     Every byte goes back to zero when the profile stops or VBind exits, and
     VBind zeroes it at start-up in case it crashed.
   - `controls`: one per game action, by id: `label`, `kind` (`axis` -1..1,
     `trigger` 0..1, `button`), the byte offset `at`, the little-endian
     `type` (`u8`..`u64`, `i8`..`i64`, `f32`, `f64`), `bit` for buttons
     packed into an integer, `range` for integer axes, and `pulse_ms` for
     actions the add-on only notices on a rising edge (VBind keeps the bit
     set that long so a quick tap is not missed). `group` and `description`
     help the person.
   - `channels`: optional JSON documents in shared memory: `status` (with
     the JSON pointers to its state and timestamp, and the states that mean
     "working"), `telemetry` (shown through `readouts`), and `settings`
     (VBind writes the profile's settings there).
   - `settings`: what the add-on accepts, each with a `default` and
     `choices` or `min`/`max`.
   - `game`: `name`, `steam_app_id`, and `tested_builds` (Steam build ids
     the add-on is known to work with).
   - `install`: steps that put the add-on in place, all inside
     `{game_dir}` (the game's Steam folder): `{"action": "copy", "file":
     "Addon.dll", "to": "{game_dir}\\Bin"}` and `{"action": "write_text",
     "path": ..., "text": ...}`. Any file a step replaces is kept and put back
     on uninstall.
   - `launch`: extra launch arguments; the game starts through Steam.
   - `source`: where the add-on came from, its version and license.
3. Put it in a folder with the add-on files under `files\`, then
   `vbind integration import <folder>` (`import_integration`). VBind checks
   that no two controls share a byte or bit, that everything fits the frame,
   and that every file is there.
4. Ask the person before `vbind integration install <id>`
   (`install_integration`): it changes files in the game folder. The game
   must be closed. After a game update, `list_integrations` shows whether the
   files are still current and whether the build was tested.
5. Map to it from a profile:

```json
"outputs": {
  "game": { "type": "integration", "integration": "space-engineers-2",
            "settings": { "flightModelMode": "DirectAngularFlight" } }
},
"map": { "game.pitch": "deadzone(left.y, 0.05)", "game.dampeners": "right.b2",
         "game.settings.translationControlMode": "toggle(left.b11)" }
```

6. `vbind integration launch <id>` (`launch_game`) starts the game with the
   add-on loaded. `get_integration` shows the add-on's live status (active,
   not running, or its error) and telemetry, so you can confirm input
   arrives without asking the person to describe what they see.

Respect the add-on's license and the game's rules. Add-ons that hook a game
can break after game updates, and may not be allowed in every multiplayer
setting; say so when you suggest one.

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
- `vbind layout render <layout> --view <view>` shows a device picture as the
  window will draw it, with every placed control numbered.
- `vbind integration show <id>` shows whether a game integration's add-on is
  running in the game, its telemetry, and (in `vbind state`) the values sent.
- Drivers: if `xbox360` is unavailable, ViGEmBus is missing
  (https://github.com/nefarius/ViGEmBus/releases). If `vjoy` is unavailable,
  install and configure vJoy.
