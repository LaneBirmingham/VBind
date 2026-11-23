// Every connected game controller with live readings, its layout card drawing
// and, for the profile on the board, what each control does.

import type { Device, Graph, Layout } from "../api";
import { graph, layouts, live, outputInputs, panel, readControl } from "../state";
import { DeviceDrawing } from "./DeviceDrawing";
import { LiveViews } from "./Figure";
import { IntegrationsSection } from "./Integrations";
import { fmt, Lamp, Meter } from "./bits";
import { shortName } from "./WiringBoard";

const AXIS_LABEL: Record<string, string> = {
  x: "X", y: "Y", z: "Z", rx: "Rx", ry: "Ry", rz: "Rz", slider1: "Slider 1", slider2: "Slider 2",
};
const AXES = ["x", "y", "z", "rx", "ry", "rz", "slider1", "slider2"];
const HAT_NAMES: [number, string][] = [
  [0, "up"], [4500, "up-right"], [9000, "right"], [13500, "down-right"], [18000, "down"], [22500, "down-left"], [27000, "left"], [31500, "up-left"],
];

function hatName(angle: number | null | undefined): string {
  if (angle == null) return "centred";
  let best = HAT_NAMES[0];
  for (const h of HAT_NAMES) {
    const d = Math.min(Math.abs(angle - h[0]), 36000 - Math.abs(angle - h[0]));
    const b = Math.min(Math.abs(angle - best[0]), 36000 - Math.abs(angle - best[0]));
    if (d < b) best = h;
  }
  return best[1];
}

export function DevicesView({ devices }: { devices: Device[] }) {
  const resp = graph.value;
  return (
    <div class="page">
      <h1>Devices</h1>
      <p class="muted">
        Game controllers Windows can see. Move a stick or press a button to find which card is which. VBind hides its own virtual
        controllers from this list.
      </p>
      {!devices.length && (
        <div class="empty" style={{ margin: "40px auto" }}>
          <h1>No controllers found</h1>
          <p>Plug in a joystick, throttle, wheel or gamepad. It shows up here within a second or two.</p>
        </div>
      )}
      <div class="grid">
        {devices.map((d) => (
          <DeviceCard d={d} layout={layouts.value.find((l) => l.id === d.layout)} g={resp?.graph} role={resp?.devices.find((x) => x.device?.instance === d.instance)?.role} />
        ))}
      </div>
      <IntegrationsSection />
    </div>
  );
}

function DeviceCard({ d, layout, g, role }: { d: Device; layout?: Layout; g?: Graph; role?: string }) {
  const s = live.value?.devices[d.instance];
  const pressed = new Set(s?.buttons ?? []);
  const anyOn = pressed.size > 0 || (s?.hats ?? []).some((h) => h != null);
  const hl = panel.value.highlights.filter((h) => h.device === d.instance || (role && h.role === role));
  const label = (key: string, fallback: string) => layout?.controls[key]?.label ?? fallback;
  const roleInfo = g?.roles.find((r) => r.role === role);

  // What each control does in the board's profile, in the profile's control order.
  const does = new Map<string, { control: string; label: string; to: string[] }>();
  if (g && role) {
    for (const ctl of roleInfo?.controls ?? []) does.set(ctl.control, { control: ctl.control, label: ctl.label, to: [] });
    for (const o of g.outputs)
      for (const c of o.controls)
        for (const r of uniq(outputInputs(g, c)))
          if (r.role === role)
            does.get(r.control)?.to.push(`${g.outputs.length > 1 ? `${o.label} ` : ""}${shortName(c.control, c.label)}${c.action ? ` - ${c.action}` : ""}`);
  }
  const doesList = [...does.values()].filter((x) => x.to.length);
  const usedKeys = new Set(doesList.map((x) => layoutKey(x.control)));

  return (
    <div class="dcard" style={anyOn ? { borderColor: "var(--accent-fill)" } : undefined}>
      <div class="top">
        <div style={{ flex: 1, minWidth: 0 }}>
          <div class="name">{d.name}</div>
          <div class="meta">
            {d.vid && d.pid ? `${d.vid}:${d.pid}` : "no USB id"} - {d.axes.length} axes, {d.buttons} buttons, {d.hats} hats
          </div>
        </div>
        {roleInfo && <span class="chip">{roleInfo.label}</span>}
        {!roleInfo && d.roles.length > 0 && <span class="chip">{d.roles.map((r) => r.role).join(", ")}</span>}
      </div>

      {layout && layout.views?.length ? (
        <LiveViews layout={layout} isOn={(k) => isOn(s, k)} mapped={(k) => usedKeys.has(k) || hl.some((h) => layoutKey(h.control) === k)} />
      ) : layout ? (
        <DeviceDrawing
          layout={layout}
          isOn={(k) => isOn(s, k)}
          mapped={(k) => usedKeys.has(k) || hl.some((h) => layoutKey(h.control) === k)}
        />
      ) : null}

      {d.axes.length > 0 && (
        <div>
          <div class="section-title" style={{ marginBottom: 6 }}>
            Axes
          </div>
          <div class="axes">
            {d.axes.map((a) => {
              const v = s?.axes[AXES.indexOf(a)] ?? 0;
              return (
                <>
                  <span title={a}>{label(a, AXIS_LABEL[a] ?? a)}</span>
                  <Meter value={v} centered />
                  <span class="mono small" style={{ textAlign: "right" }}>
                    {fmt(v)}
                  </span>
                </>
              );
            })}
          </div>
        </div>
      )}

      {d.hats > 0 && (
        <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
          {Array.from({ length: d.hats }, (_, i) => (
            <span class="small" style={{ display: "flex", gap: 8, alignItems: "center" }}>
              <Lamp on={s?.hats[i] != null} />
              {label(`hat${i + 1}`, `Hat ${i + 1}`)}: <span class="mono">{hatName(s?.hats[i])}</span>
            </span>
          ))}
        </div>
      )}

      {d.buttons > 0 && (
        <div>
          <div class="section-title" style={{ marginBottom: 6 }}>
            Buttons
          </div>
          <div class="buttons-grid">
            {Array.from({ length: d.buttons }, (_, i) => i + 1).map((b) => (
              <span class={`bcell${pressed.has(b) ? " on" : ""}`} title={label(`b${b}`, `Button ${b}`)}>
                {b}
              </span>
            ))}
          </div>
        </div>
      )}

      {doesList.length > 0 && (
        <div>
          <div class="section-title" style={{ marginBottom: 6 }}>
            What it does
          </div>
          <div class="does">
            {doesList.map((x) => (
              <div class={`r${Math.abs(readControl(s, x.control)) > 0.06 ? " on" : ""}`}>
                <span>{x.label}</span>
                <span class="to">{x.to.join(", ")}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function uniq<T extends { role: string; control: string }>(xs: T[]): T[] {
  return [...new Map(xs.map((x) => [`${x.role}.${x.control}`, x])).values()];
}

/** Layout cards label a whole hat as `hat1`; expressions use `hat1.up`. */
function layoutKey(control: string): string {
  const h = /^(hat\d+)\./.exec(control);
  return h ? h[1] : control;
}

function isOn(s: ReturnType<typeof liveDevice>, key: string): boolean {
  if (!s) return false;
  const h = /^hat(\d+)$/.exec(key);
  if (h) return s.hats[Number(h[1]) - 1] != null;
  const v = readControl(s, key);
  return AXES.includes(key) ? Math.abs(v) > 0.5 : v >= 0.5;
}

function liveDevice(guid: string) {
  return live.value?.devices[guid];
}
