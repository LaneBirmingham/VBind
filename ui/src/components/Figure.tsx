// Device pictures with callouts: each mapped control points to the game
// button it drives. Pictures come from the device's layout card (views drawn
// by an agent, optionally over the person's photo); without views, the
// generic shape and positions are used.

import { useMemo, useRef, useState } from "preact/hooks";
import { api, apiUrl, type DrawingView, type Graph, type GraphRef, type Layout, type LayoutControl } from "../api";
import { drawings, errorText, layouts, notify, outputInputs, panel, roleInput } from "../state";
import { SHAPES } from "./DeviceDrawing";
import { type GlyphSpec, glyphFor, Glyphs, mergeGlyphs, type OutKind } from "./Glyph";

type OutCtl = Graph["outputs"][number]["controls"][number];

export interface Link {
  role: string;
  /** Full input control id: `b1`, `y`, `hat1.up`. */
  control: string;
  out: OutCtl;
  outKind: OutKind;
}

/** One physical control and everything it drives. */
export interface Callout {
  role: string;
  /** Layout key: `b1`, `y`, `hat1`. */
  key: string;
  links: Link[];
  glyphs: GlyphSpec[];
  action: string;
  actions: string[];
  hints: string[];
  graphLabel: string;
  axis: boolean;
}

export function layoutKey(control: string): string {
  const h = /^(hat\d+)\./.exec(control);
  return h ? h[1] : control;
}

const AXES = new Set(["x", "y", "z", "rx", "ry", "rz", "slider1", "slider2"]);

/** "Ability 1, Ability 2, Ability 3, Ability 4" reads as "Ability 1-4". */
export function compactActions(actions: string[]): string {
  if (actions.length > 1) {
    const parts = actions.map((a) => /^(.*?)(\d+)$/.exec(a));
    if (parts.every((p) => p && p[1] === parts[0]![1])) {
      const nums = parts.map((p) => Number(p![2])).sort((a, b) => a - b);
      if (nums.every((n, i) => i === 0 || n === nums[i - 1] + 1)) return `${parts[0]![1]}${nums[0]}-${nums[nums.length - 1]}`;
    }
  }
  return actions.join(", ");
}

/** Callouts by role: what every physical control used by the profile drives. */
export function buildCallouts(g: Graph): Map<string, Callout[]> {
  const byRole = new Map<string, Map<string, Callout>>();
  for (const o of g.outputs)
    for (const c of o.controls) {
      const seen = new Set<string>();
      for (const r of outputInputs(g, c)) {
        const id = `${r.role}.${r.control}`;
        if (seen.has(id)) continue;
        seen.add(id);
        const key = layoutKey(r.control);
        let m = byRole.get(r.role);
        if (!m) byRole.set(r.role, (m = new Map()));
        let co = m.get(key);
        if (!co) {
          const gl = g.roles.find((x) => x.role === r.role)?.controls.find((x) => x.control === r.control)?.label ?? r.control;
          m.set(key, (co = { role: r.role, key, links: [], glyphs: [], action: "", actions: [], hints: [], graphLabel: gl, axis: AXES.has(key) }));
        }
        co.links.push({ role: r.role, control: r.control, out: c, outKind: o.kind });
      }
    }
  const out = new Map<string, Callout[]>();
  for (const [role, m] of byRole) {
    const list = [...m.values()];
    for (const co of list) {
      // Integration controls have no button art; their name already shows as the action.
      co.glyphs = mergeGlyphs(co.links.filter((l) => l.outKind !== "integration").map((l) => glyphFor(l.outKind, l.out.control, l.out.label)));
      co.actions = [...new Set(co.links.map((l) => l.out.action ?? l.out.label))];
      co.action = compactActions(co.actions);
      co.hints = [...new Set(co.links.map((l) => l.out.hint).filter((h): h is string => !!h))];
    }
    out.set(role, list);
  }
  return out;
}

/** Name of a control on the person's device, with the hat direction when only one is used. */
export function calloutLabel(co: Callout, layout: Layout | null): string {
  const dirs = [...new Set(co.links.map((l) => l.control.split(".")[1]).filter(Boolean))];
  const base = layout?.controls[co.key]?.label;
  if (base) return dirs.length === 1 ? `${base} ${dirs[0].replace("_", "-")}` : base;
  // Graph labels name the direction; drop it when several directions are used.
  if (co.key.startsWith("hat") && dirs.length !== 1) return co.graphLabel.replace(/ (up|down)?-?(left|right)?$/, "");
  return co.graphLabel;
}

export function isLinkOn(l: { role: string; control: string }): boolean {
  // Sliders and throttles rest at an end, so their position says nothing about use.
  if (l.control.startsWith("slider")) return false;
  const v = roleInput(l.role, l.control);
  return AXES.has(l.control) ? Math.abs(v) > 0.25 : v > 0.5;
}

/** The layout card for a role's device, from the graph. */
export function layoutFor(gr: { devices: { role: string; layout: string | null }[] } | null, role: string): Layout | null {
  const id = gr?.devices.find((d) => d.role === role)?.layout;
  return (id && layouts.value.find((l) => l.id === id)) || null;
}

interface PlacedView {
  view: DrawingView;
  /** Real layout view (false for the generic shape). */
  real: boolean;
  at: (key: string, c: LayoutControl) => [number, number] | null;
}

/** The views to draw for a layout: its own, or the generic shape with `pos`. */
export function viewsFor(layout: Layout | null): PlacedView[] {
  if (!layout) return [];
  const d = drawings.value[layout.id];
  if (d?.views.length) {
    const only = d.views.length === 1 ? d.views[0].id : null;
    return d.views.map((v) => ({ view: v, real: true, at: (_k, c) => (c.at && (c.view ?? only) === v.id ? c.at : null) }));
  }
  if (Object.values(layout.controls).some((c) => c.pos)) {
    const paths = SHAPES[layout.shape ?? "generic"] ?? SHAPES.generic;
    return [{ view: { id: "_shape", label: "Outline", size: [100, 100], shapes: paths.map((d) => ({ d, style: "body" })) }, real: false, at: (_k, c) => c.pos ?? null }];
  }
  return [];
}

const BOX_W = 244;
const GAP = 30;
const PAD = 10;

export type FigureMode = "drawing" | "photo";

/** One view with markers and callouts. */
export function ViewFigure({
  layout,
  pv,
  callouts,
  height,
  editing,
  mode,
  highlight,
  onPick,
  showCallouts = true,
}: {
  layout: Layout;
  pv: PlacedView;
  callouts: Callout[];
  height: number;
  editing?: boolean;
  mode: FigureMode;
  highlight?: string | null;
  onPick?: (co: Callout, what: "callout" | "marker") => void;
  showCallouts?: boolean;
}) {
  const { view } = pv;
  const svgRef = useRef<SVGSVGElement>(null);
  const [drag, setDrag] = useState<{ key: string; at: [number, number] } | null>(null);
  const [w, h] = view.size;
  const s = Math.min(height / h, (height * 1.1) / w);
  const dw = w * s;
  const dh = h * s;
  const markerR = (c?: LayoutControl) => Math.max(7, Math.min(26, (c?.size ?? Math.max(w, h) * 0.026) * s));
  const posOf = (key: string): [number, number] | null => {
    if (drag?.key === key) return drag.at;
    const c = layout.controls[key];
    return c ? pv.at(key, c) : null;
  };

  const placed = callouts.filter((co) => posOf(co.key));
  const hlPanel = panel.value.highlights;
  const items = placed.map((co) => {
    const [x, y] = posOf(co.key)!;
    const on = co.links.some(isLinkOn) || hlPanel.some((hl) => hl.role === co.role && layoutKey(hl.control) === co.key) || highlight === co.key;
    return { co, x: x * s, y: y * s, left: x < w / 2, h: co.hints.length ? 56 : 42, on, ly: 0 };
  });
  // Stack callouts per side, in marker order, without overlaps.
  let bottom = dh;
  for (const left of [true, false]) {
    const side = items.filter((i) => i.left === left).sort((a, b) => a.y - b.y);
    let next = -Infinity;
    for (const it of side) {
      it.ly = Math.max(it.y - it.h / 2, next);
      next = it.ly + it.h + 6;
    }
    let limit = Math.max(dh, next - 6);
    for (let i = side.length - 1; i >= 0; i--) {
      if (side[i].ly + side[i].h > limit) side[i].ly = limit - side[i].h;
      limit = side[i].ly - 6;
    }
    const shift = side.length ? Math.max(0, -side[0].ly) : 0;
    for (const it of side) it.ly += shift;
    bottom = Math.max(bottom, ...side.map((i) => i.ly + i.h));
  }
  const hasL = showCallouts && items.some((i) => i.left);
  const hasR = showCallouts && items.some((i) => !i.left);
  const GL = hasL ? BOX_W + GAP : 18;
  const GR = hasR ? BOX_W + GAP : 18;
  const W = GL + dw + GR;
  const H = (showCallouts ? bottom : dh) + PAD * 2;

  // In edit mode every placed control can be dragged, mapped or not.
  const editable = editing && pv.real ? Object.entries(layout.controls).filter(([k]) => posOf(k)) : [];

  const toView = (e: PointerEvent): [number, number] => {
    const svg = svgRef.current!;
    const pt = svg.createSVGPoint();
    pt.x = e.clientX;
    pt.y = e.clientY;
    const p = pt.matrixTransform(svg.getScreenCTM()!.inverse());
    const r = (v: number) => Math.round(v * 10) / 10;
    return [r(Math.max(0, Math.min(w, (p.x - GL) / s))), r(Math.max(0, Math.min(h, (p.y - PAD) / s)))];
  };
  const save = async (key: string, at: [number, number]) => {
    try {
      await api.post(`/layouts/${encodeURIComponent(layout.id)}/place`, { view: view.id, controls: { [key]: at } });
    } catch (e) {
      notify(errorText(e), "error");
    }
  };

  const photo = mode === "photo" ? view.photo : undefined;
  const clipId = `clip-${layout.id}-${view.id}`;
  return (
    <svg
      ref={svgRef}
      class={`figure${editing ? " editing" : ""}`}
      width={W}
      height={H}
      viewBox={`0 0 ${W} ${H}`}
      role="img"
      aria-label={`${layout.name}, ${view.label}`}
      onPointerMove={(e) => drag && setDrag({ key: drag.key, at: toView(e as unknown as PointerEvent) })}
      onPointerUp={() => {
        if (drag) {
          save(drag.key, drag.at);
          setTimeout(() => setDrag(null), 400);
        }
      }}
    >
      <g transform={`translate(${GL} ${PAD}) scale(${s})`}>
        {photo && (
          <>
            <clipPath id={clipId}>
              <rect width={w} height={h} />
            </clipPath>
            <image href={apiUrl(photo.url)} x={photo.fit[0]} y={photo.fit[1]} width={photo.fit[2]} height={photo.fit[3]} preserveAspectRatio="none" clip-path={`url(#${clipId})`} />
          </>
        )}
        {view.shapes.map((sh) => (
          <path d={sh.d} class={`sh sh-${sh.style}${photo ? " over-photo" : ""}`} fill-rule="evenodd" vector-effect="non-scaling-stroke" />
        ))}
      </g>
      {showCallouts &&
        items.map((it) => {
          const mx = GL + it.x;
          const my = PAD + it.y;
          const bx = it.left ? GL - GAP - BOX_W : GL + dw + GAP;
          const edge = it.left ? GL - GAP : GL + dw + GAP;
          const cy = PAD + it.ly + 20;
          return (
            <g class={`callout${it.on ? " on" : ""}`} key={`${it.co.role}.${it.co.key}`}>
              <polyline class="leader" points={`${mx},${my} ${it.left ? edge + 10 : edge - 10},${cy} ${edge},${cy}`} />
              <foreignObject x={bx} y={PAD + it.ly} width={BOX_W} height={it.h}>
                <button class={`co${it.left ? " left" : ""}${it.on ? " on" : ""}`} onClick={() => onPick?.(it.co, "callout")} title={calloutTitle(it.co, layout)}>
                  <span class="co-g">
                    <Glyphs glyphs={it.co.glyphs} size={12} on={it.on} />
                  </span>
                  <span class="co-t">
                    <span class="co-a">{it.co.action}</span>
                    <span class="co-c">{calloutLabel(it.co, layout)}</span>
                    {it.co.hints.length > 0 && <span class="co-h">{it.co.hints.join("; ")}</span>}
                  </span>
                </button>
              </foreignObject>
            </g>
          );
        })}
      {!editing &&
        items.map((it) => (
          <circle
            class={`mk${it.on ? " on" : ""}`}
            cx={GL + it.x}
            cy={PAD + it.y}
            r={markerR(layout.controls[it.co.key])}
            onClick={() => onPick?.(it.co, "marker")}
          >
            <title>{calloutLabel(it.co, layout)}</title>
          </circle>
        ))}
      {editable.map(([key, c]) => {
        const [x, y] = posOf(key)!;
        const mapped = callouts.some((co) => co.key === key);
        return (
          <g class={`mk-edit${mapped ? " mapped" : ""}${drag?.key === key ? " dragging" : ""}`}>
            <circle
              cx={GL + x * s}
              cy={PAD + y * s}
              r={markerR(c)}
              onPointerDown={(e) => {
                (e.currentTarget as Element).setPointerCapture((e as PointerEvent).pointerId);
                setDrag({ key, at: [x, y] });
              }}
            >
              <title>{`${c.label} (${key}): drag to move`}</title>
            </circle>
            <text x={GL + x * s} y={PAD + y * s - markerR(c) - 4} text-anchor="middle">
              {key}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

function calloutTitle(co: Callout, layout: Layout): string {
  const lines = co.links.map((l) => `${l.control}: ${l.out.action ?? l.out.label} (${l.out.key})`);
  return `${calloutLabel(co, layout)}\n${lines.join("\n")}${co.hints.length ? `\nHint: ${co.hints.join("; ")}` : ""}`;
}

export function refsOf(co: Callout): GraphRef[] {
  return co.links.map((l) => ({ role: l.role, control: l.control }));
}

/** A device's pictures with every placed control lit while held, for the Devices tab. */
export function LiveViews({ layout, isOn, mapped }: { layout: Layout; isOn: (key: string) => boolean; mapped: (key: string) => boolean }) {
  const views = viewsFor(layout).filter((pv) => pv.real);
  if (!views.length) return null;
  return (
    <div class="live-views">
      {views.map((pv) => {
        const [w, h] = pv.view.size;
        const s = Math.min(220 / h, 260 / w);
        const pts = Object.entries(layout.controls).flatMap(([k, c]) => {
          const at = pv.at(k, c);
          return at ? [{ k, c, at }] : [];
        });
        return (
          <figure>
            <svg class="figure" width={w * s + 12} height={h * s + 12} viewBox={`-6 -6 ${w * s + 12} ${h * s + 12}`} role="img" aria-label={`${layout.name}, ${pv.view.label}`}>
              <g transform={`scale(${s})`}>
                {pv.view.shapes.map((sh) => (
                  <path d={sh.d} class={`sh sh-${sh.style}`} fill-rule="evenodd" vector-effect="non-scaling-stroke" />
                ))}
              </g>
              {pts.map(({ k, c, at }) => (
                <circle
                  class={`mk small${isOn(k) && !k.startsWith("slider") ? " on" : ""}${mapped(k) ? "" : " unmapped"}`}
                  cx={at[0] * s}
                  cy={at[1] * s}
                  r={Math.max(4, Math.min(16, (c.size ?? Math.max(w, h) * 0.026) * s))}
                >
                  <title>{`${c.label} (${k})`}</title>
                </circle>
              ))}
            </svg>
            <figcaption class="section-title">{pv.view.label}</figcaption>
          </figure>
        );
      })}
    </div>
  );
}
