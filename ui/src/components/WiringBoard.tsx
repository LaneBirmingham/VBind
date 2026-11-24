// The wiring board: physical controls on the left, variables (mixers) in the
// middle, virtual outputs on the right, with a live wire for every mapping.

import { useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import type { Graph } from "../api";
import { deviceName, drawerOpen, graph, live, panel, roleInput, roleTone, selection, sideTab } from "../state";
import { fmt, Lamp, Meter, TONES } from "./bits";
import { AddonStatusText } from "./Integrations";

type PortKey = string; // in:role.control | var:name | out:key

interface Edge {
  from: PortKey;
  to: PortKey;
  tone: number | "mix";
}

interface Wire extends Edge {
  d: string;
}

const COL_IN = 280;
const COL_MID = 210;
const COL_OUT = 300;

function edgesOf(g: Graph): Edge[] {
  const edges: Edge[] = [];
  const tone = (role: string) => Math.max(0, g.roles.findIndex((r) => r.role === role)) % 4;
  for (const v of g.vars) {
    for (const r of v.inputs) edges.push({ from: `in:${r.role}.${r.control}`, to: `var:${v.name}`, tone: tone(r.role) });
    for (const d of v.vars) edges.push({ from: `var:${d}`, to: `var:${v.name}`, tone: "mix" });
  }
  for (const o of g.outputs)
    for (const c of o.controls) {
      for (const r of c.inputs) edges.push({ from: `in:${r.role}.${r.control}`, to: `out:${c.key}`, tone: tone(r.role) });
      for (const d of c.vars) edges.push({ from: `var:${d}`, to: `out:${c.key}`, tone: "mix" });
    }
  return edges;
}

/** Order output rows by the average position of what feeds them, to keep wires short. */
function orderOutputs(g: Graph) {
  const rank = new Map<string, number>();
  let i = 0;
  for (const r of g.roles) for (const c of r.controls) rank.set(`in:${r.role}.${c.control}`, i++);
  const mean = (xs: number[], fallback: number) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : fallback);
  for (const v of [...g.vars].sort((a, b) => a.depth - b.depth)) {
    const src = [...v.inputs.map((r) => rank.get(`in:${r.role}.${r.control}`) ?? 0), ...v.vars.map((d) => rank.get(`var:${d}`) ?? 0)];
    rank.set(`var:${v.name}`, mean(src, 0));
  }
  return g.outputs.map((o) => ({
    ...o,
    controls: o.controls
      .map((c, idx) => {
        const src = [...c.inputs.map((r) => rank.get(`in:${r.role}.${r.control}`) ?? 0), ...c.vars.map((d) => rank.get(`var:${d}`) ?? 0)];
        return { c, idx, bary: mean(src, 1e9) };
      })
      .sort((a, b) => a.bary - b.bary || a.idx - b.idx)
      .map((x) => x.c),
  }));
}

/** Nodes connected to the seeds: everything downstream, and upstream if asked. */
function connected(edges: Edge[], seeds: string[], upstream: boolean): Set<string> {
  const keep = new Set<string>();
  const walk = (dir: "up" | "down") => {
    const stack = [...seeds];
    const seen = new Set<string>();
    while (stack.length) {
      const n = stack.pop()!;
      if (seen.has(n)) continue;
      seen.add(n);
      keep.add(n);
      for (const e of edges) {
        if (dir === "up" && e.to === n) stack.push(e.from);
        if (dir === "down" && e.from === n) stack.push(e.to);
      }
    }
  };
  walk("down");
  if (upstream) walk("up");
  return keep;
}

export function selectionKey(sel: string): PortKey {
  return sel.startsWith("var:") || sel.startsWith("in:") ? sel : `out:${sel}`;
}

export function WiringBoard({ onAddMapping }: { onAddMapping: (output: string) => void }) {
  const resp = graph.value;
  const g = resp?.graph;
  const boardRef = useRef<HTMLDivElement>(null);
  const ports = useRef(new Map<PortKey, HTMLElement>());
  const nodeRefs = useRef(new Map<string, HTMLElement>());
  const [tops, setTops] = useState<Record<string, number>>({});
  const [midHeight, setMidHeight] = useState(0);
  const [wires, setWires] = useState<Wire[]>([]);
  const [size, setSize] = useState(0);
  const [focus, setFocus] = useState<string | null>(null);
  const sel = selection.value;

  const edges = useMemo(() => (g ? edgesOf(g) : []), [g]);
  const outputs = useMemo(() => (g ? orderOutputs(g) : []), [g]);
  const depths = useMemo(() => (g && g.vars.length ? Math.max(...g.vars.map((v) => v.depth)) : 0), [g]);
  const keep = useMemo(() => {
    if (!g) return null;
    if (sel && !sel.startsWith("new:")) return connected(edges, [selectionKey(sel)], true);
    if (focus) {
      const role = g.roles.find((r) => r.role === focus);
      return connected(edges, role ? role.controls.map((c) => `in:${focus}.${c.control}`) : [], false);
    }
    return null;
  }, [g, edges, sel, focus]);
  const related = (k: PortKey) => !keep || keep.has(k);

  // Re-measure when the board or any mixer changes size (fonts loading, text wrapping).
  useLayoutEffect(() => {
    const el = boardRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize((s) => s + 1));
    ro.observe(el);
    for (const n of nodeRefs.current.values()) ro.observe(n);
    return () => ro.disconnect();
  }, [g]);

  // Place each variable level with the controls that feed it, pushing down to avoid overlap.
  useLayoutEffect(() => {
    const board = boardRef.current;
    if (!board || !g) return;
    const top0 = board.getBoundingClientRect().top;
    const centerY = (k: PortKey) => {
      const el = ports.current.get(k);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return r.top + r.height / 2 - top0;
    };
    const next: Record<string, number> = {};
    const centers: Record<string, number> = {};
    let bottom = 0;
    for (let depth = 1; depth <= depths; depth++) {
      const wanted = g.vars
        .filter((v) => v.depth === depth)
        .map((v) => {
          const ys = [...v.inputs.map((r) => centerY(`in:${r.role}.${r.control}`)), ...v.vars.map((d) => centers[d] ?? null)].filter(
            (y): y is number => y !== null,
          );
          const h = nodeRefs.current.get(v.name)?.offsetHeight ?? 70;
          return { v, h, y: ys.length ? ys.reduce((a, b) => a + b, 0) / ys.length : 40 };
        })
        .sort((a, b) => a.y - b.y);
      let cursor = 28;
      for (const w of wanted) {
        const t = Math.max(cursor, w.y - w.h / 2);
        next[w.v.name] = t;
        centers[w.v.name] = t + w.h / 2;
        cursor = t + w.h + 14;
        bottom = Math.max(bottom, cursor);
      }
    }
    setTops(next);
    setMidHeight(bottom);
  }, [g, size, depths]);

  // Draw wires between measured ports.
  useLayoutEffect(() => {
    const board = boardRef.current;
    if (!board || !g) return;
    const b = board.getBoundingClientRect();
    const at = (k: PortKey) => {
      const el = ports.current.get(k);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: r.left + r.width / 2 - b.left, y: r.top + r.height / 2 - b.top };
    };
    const out: Wire[] = [];
    for (const e of edges) {
      const p = at(e.from.startsWith("var:") ? `${e.from}:out` : e.from);
      const q = at(e.to.startsWith("var:") ? `${e.to}:in` : e.to);
      if (!p || !q) continue;
      const dx = Math.max(36, (q.x - p.x) / 2);
      out.push({ ...e, d: `M${p.x} ${p.y} C${p.x + dx} ${p.y} ${q.x - dx} ${q.y} ${q.x} ${q.y}` });
    }
    setWires(out);
  }, [g, tops, size, edges, focus]);

  if (!g || !resp) return null;
  const l = live.value;
  const active = l?.active?.profile === g.profile;
  const vars = active ? l!.active!.vars : {};
  const values = active ? l!.active!.values : {};
  const hl = panel.value.highlights;

  const inputOf = (k: PortKey) => {
    const [role, ...rest] = k.slice(3).split(".");
    return roleInput(role, rest.join("."));
  };
  const srcOn = (k: PortKey): boolean => {
    if (k.startsWith("in:")) return Math.abs(inputOf(k)) > 0.06;
    if (k.startsWith("var:")) return active && Math.abs(vars[k.slice(4)] ?? 0) > 0.02;
    return false;
  };
  const isHl = (role: string | null, control: string) =>
    hl.some((h) => (role ? h.role === role && h.control === control : !h.role && !h.device && h.control === control));
  const setPort = (k: PortKey) => (el: HTMLElement | null) => {
    if (el) ports.current.set(k, el);
    else ports.current.delete(k);
  };
  const pick = (k: string) => {
    selection.value = selection.value === k ? null : k;
    if (selection.value) {
      drawerOpen.value = true;
      sideTab.value = "inspect";
    }
  };

  // Columns start at their minimum and share any spare width up to a cap, so wide
  // windows give mixer expressions and output names room instead of empty floor.
  const columns = [
    `minmax(${COL_IN}px, ${COL_IN * 1.5}px)`,
    ...Array.from({ length: depths }, () => `minmax(${COL_MID}px, ${COL_MID * 1.75}px)`),
    `minmax(${COL_OUT}px, ${COL_OUT * 1.5}px)`,
  ].join(" ");

  return (
    <div>
      {g.roles.length > 1 && (
        <div class="filters" role="group" aria-label="Show one device">
          <button class="filter" aria-pressed={focus === null} onClick={() => setFocus(null)}>
            All devices
          </button>
          {g.roles.map((r) => (
            <button class="filter" aria-pressed={focus === r.role} onClick={() => setFocus(focus === r.role ? null : r.role)}>
              <span class="dot" style={{ background: TONES[roleTone(r.role)] }} /> {r.label}
            </button>
          ))}
        </div>
      )}
      <div class="board" ref={boardRef} style={{ gridTemplateColumns: columns }}>
        <svg class="wires" aria-hidden="true">
          {wires.map((w) => {
            const on = srcOn(w.from);
            const show = related(w.from) && related(w.to);
            const cls = w.tone === "mix" ? "mix" : `tone-${w.tone}`;
            return <path class={`wire ${cls}${on ? " on" : ""}${show ? "" : " dim"}`} d={w.d} style={w.tone === "mix" ? undefined : { color: TONES[w.tone as number] }} />;
          })}
        </svg>

        <div class="col">
          <div class="col-title">Physical devices</div>
          {g.roles.map((r) => {
            const dev = resp.devices.find((d) => d.role === r.role);
            const tone = roleTone(r.role);
            const status = dev?.resolution.status;
            return (
              <div class="dev-card" style={{ opacity: focus && focus !== r.role ? 0.45 : 1 }}>
                <div class="head">
                  <span class="tone-bar" style={{ background: TONES[tone] }} />
                  <div style={{ minWidth: 0 }}>
                    <div class="name">{r.label}</div>
                    <div class="sub">
                      {dev?.device ? (
                        deviceName(dev.device.instance) || dev.device.name
                      ) : (
                        <span class="warn-text">
                          {status === "ambiguous" ? "Several match: pick one in Profiles" : dev?.resolution.optional ? "Not connected (optional)" : "Not connected"}
                        </span>
                      )}
                    </div>
                  </div>
                </div>
                {r.controls.map((c) => {
                  const k = `in:${r.role}.${c.control}`;
                  const v = roleInput(r.role, c.control);
                  const on = Math.abs(v) > 0.06;
                  return (
                    <div
                      class={`row${on ? " on" : ""}${sel === k ? " selected" : ""}${isHl(r.role, c.control) ? " hl" : ""}`}
                      style={{ opacity: related(k) ? 1 : 0.4 }}
                      onClick={() => pick(k)}
                      title={`${r.role}.${c.control}`}
                    >
                      <span class="label">
                        {c.label}
                        <span class="id">{c.control}</span>
                      </span>
                      {c.digital ? <Lamp on={on} /> : <Meter value={v} centered tone={TONES[tone]} />}
                      <span class="port right" ref={setPort(k)} />
                    </div>
                  );
                })}
                {!r.controls.length && <div class="row muted small">Nothing mapped from this device yet</div>}
              </div>
            );
          })}
        </div>

        {Array.from({ length: depths }, (_, i) => i + 1).map((depth) => (
          <div class="col mid" style={{ height: `${midHeight}px` }}>
            <div class="col-title">{depth === 1 ? "Mixing" : " "}</div>
            {g.vars
              .filter((v) => v.depth === depth)
              .map((v) => {
                const val = vars[v.name] ?? 0;
                const on = active && Math.abs(val) > 0.02;
                const k = `var:${v.name}`;
                return (
                  <div
                    class={`node${on ? " on" : ""}${sel === k ? " selected" : ""}`}
                    style={{ top: `${tops[v.name] ?? 28}px`, opacity: related(k) ? 1 : 0.4 }}
                    ref={(el) => {
                      if (el) nodeRefs.current.set(v.name, el);
                      else nodeRefs.current.delete(v.name);
                    }}
                    onClick={() => pick(k)}
                  >
                    <span class="port left" ref={setPort(`${k}:in`)} />
                    <div class="t">
                      {v.label ?? v.name}
                      <span class="v">{active ? fmt(val) : ""}</span>
                    </div>
                    <div class="e">{v.expr}</div>
                    {active && <Meter value={val} centered />}
                    <span class="port right" ref={setPort(`${k}:out`)} />
                  </div>
                );
              })}
          </div>
        ))}

        <div class="col">
          <div class="col-title">Virtual devices</div>
          {outputs.map((o) => {
            const ol = l?.active?.outputs.find((x) => x.name === o.name);
            return (
              <div class="dev-card out-card">
                <div class="head">
                  <span class="tone-bar" style={{ background: "var(--accent-fill)" }} />
                  <div style={{ minWidth: 0 }}>
                    <div class="name">{o.label}</div>
                    <div class="sub">{outputStatus(active, o.kind, ol, o.integration)}</div>
                  </div>
                </div>
                {o.controls.map((c) => {
                  const k = `out:${c.key}`;
                  const v = values[c.key] ?? 0;
                  const digital = c.kind === "button" || c.kind === "hat_dir";
                  const on = active && (digital ? v >= 0.5 : Math.abs(v) > 0.02);
                  return (
                    <div
                      class={`row${on ? " on" : ""}${sel === c.key ? " selected" : ""}${isHl(null, c.key) ? " hl" : ""}`}
                      style={{ opacity: related(k) ? 1 : 0.4 }}
                      onClick={() => pick(c.key)}
                      title={`${c.key} = ${c.expr}`}
                    >
                      <span class="port left" ref={setPort(k)} />
                      <span class="label">
                        <b>{shortName(c.control, c.label)}</b>
                        <span class="action">{c.action ?? ""}</span>
                      </span>
                      {digital ? <Lamp on={on} /> : <Meter value={v} centered={c.kind === "axis"} tone="var(--accent)" />}
                    </div>
                  );
                })}
                <div class="add-map">
                  <button class="btn ghost small" onClick={() => onAddMapping(o.name)}>
                    + Map another control
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function outputStatus(active: boolean, kind: string, ol: { error?: string; xinput_slot?: number; vjoy_id?: number } | undefined, integration?: string) {
  if (kind === "integration") return <AddonStatusText id={integration} active={active} error={ol?.error} />;
  if (!active) return "Off - press Start to plug it in";
  if (ol?.error) return <span class="err-text">{ol.error}</span>;
  if (kind === "xbox360") return <span class="ok-text">Plugged in{ol?.xinput_slot != null ? `, player ${ol.xinput_slot + 1}` : ""}</span>;
  return <span class="ok-text">vJoy device {ol?.vjoy_id ?? ""}</span>;
}

export function shortName(control: string, label: string): string {
  const short: Record<string, string> = {
    lx: "Left stick X", ly: "Left stick Y", rx: "Right stick X", ry: "Right stick Y", lt: "LT", rt: "RT", lb: "LB", rb: "RB",
    ls: "LS click", rs: "RS click", back: "Back", start: "Start", guide: "Guide",
    dpad_up: "D-pad up", dpad_down: "D-pad down", dpad_left: "D-pad left", dpad_right: "D-pad right",
  };
  return short[control] ?? (control.length <= 2 ? control.toUpperCase() : label);
}
