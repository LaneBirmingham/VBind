// Game-button glyphs: what a game shows on screen for a virtual control.
// Xbox face buttons are coloured letters, other buttons are labelled pills,
// and the D-pad is a cross with the mapped arms lit.

export type OutKind = "xbox360" | "vjoy" | "integration" | "keyboard";

export interface GlyphSpec {
  kind: "face" | "pill" | "dpad";
  text: string;
  color?: string;
  /** D-pad arms this glyph stands for. */
  arms?: string[];
}

const FACE: Record<string, string> = { a: "#3f9a52", b: "#c64545", x: "#3f78bd", y: "#c9971a" };
const PILL: Record<string, string> = {
  lb: "LB", rb: "RB", lt: "LT", rt: "RT", back: "VIEW", start: "MENU", guide: "GUIDE", ls: "LS", rs: "RS",
  lx: "L ↔", ly: "L ↕", rx: "R ↔", ry: "R ↕",
};
const DIRS = ["up", "down", "left", "right"];
/** Short keyboard and mouse glyphs; other keys use their label ("Left Alt", "F1"). */
const KEY_PILL: Record<string, string> = {
  "mouse.x": "Mouse ↔", "mouse.y": "Mouse ↕", "mouse.wheel": "Wheel",
  "mouse.left": "LMB", "mouse.right": "RMB", "mouse.middle": "MMB", "mouse.x1": "Mouse 4", "mouse.x2": "Mouse 5",
};

/** The glyph for one output control, for example `a`, `dpad_up` or (vJoy) `b3`.
 * Game integrations have no button art, so their glyph is the control's name. */
export function glyphFor(kind: OutKind, control: string, label?: string): GlyphSpec {
  if (kind === "integration") return { kind: "pill", text: shortPill(label ?? control) };
  if (kind === "keyboard") return { kind: "pill", text: shortPill(KEY_PILL[control] ?? label ?? control) };
  if (kind === "xbox360") {
    if (FACE[control]) return { kind: "face", text: control.toUpperCase(), color: FACE[control] };
    if (control.startsWith("dpad_")) return { kind: "dpad", text: "D-pad", arms: [control.slice(5)] };
    return { kind: "pill", text: PILL[control] ?? control.toUpperCase() };
  }
  const hat = /^hat(\d)\.(\w+)$/.exec(control);
  if (hat && DIRS.includes(hat[2])) return { kind: "dpad", text: `Hat ${hat[1]}`, arms: [hat[2]] };
  return { kind: "pill", text: control.replace(/^b/, "B").toUpperCase() };
}

/** Merge D-pad arms into one glyph and drop duplicates; keep at most `max`. */
export function mergeGlyphs(specs: GlyphSpec[], max = 4): GlyphSpec[] {
  const out: GlyphSpec[] = [];
  for (const g of specs) {
    const same = out.find((o) => o.kind === g.kind && (g.kind === "dpad" ? o.text === g.text : o.text === g.text));
    if (same) {
      if (g.arms) same.arms = [...new Set([...(same.arms ?? []), ...g.arms])];
      continue;
    }
    out.push({ ...g, arms: g.arms ? [...g.arms] : undefined });
  }
  return out.slice(0, max);
}

export function glyphWidth(g: GlyphSpec, size: number): number {
  if (g.kind === "face" || g.kind === "dpad") return size * 2;
  return Math.max(size * 2.6, g.text.length * size * 0.66 + size * 1.1);
}

/** One glyph drawn with its left edge at x, centred on y. */
export function GlyphG({ g, x, y, size = 11, on = false }: { g: GlyphSpec; x: number; y: number; size?: number; on?: boolean }) {
  const w = glyphWidth(g, size);
  if (g.kind === "face")
    return (
      <g class={`glyph face${on ? " on" : ""}`}>
        {on && <circle cx={x + size} cy={y} r={size + 4} class="glyph-glow" />}
        <circle cx={x + size} cy={y} r={size} fill={g.color} />
        <text x={x + size} y={y + size * 0.38} text-anchor="middle" style={{ fontSize: `${size * 1.05}px` }}>
          {g.text}
        </text>
      </g>
    );
  if (g.kind === "dpad") {
    const c = x + size;
    const a = size * 0.36;
    const arm = (dir: string) => {
      const lit = g.arms?.includes(dir);
      const r = dir === "up" ? [c - a, y - size, a * 2, size - a] : dir === "down" ? [c - a, y + a, a * 2, size - a] : dir === "left" ? [x, y - a, size - a, a * 2] : [c + a, y - a, size - a, a * 2];
      return <rect x={r[0]} y={r[1]} width={r[2]} height={r[3]} class={lit ? "arm lit" : "arm"} />;
    };
    const cross = `M${c - a} ${y - size} h${a * 2} v${size - a} h${size - a} v${a * 2} h${-(size - a)} v${size - a} h${-a * 2} v${-(size - a)} h${-(size - a)} v${-a * 2} h${size - a} Z`;
    return (
      <g class={`glyph dpad${on ? " on" : ""}`}>
        {on && <circle cx={c} cy={y} r={size + 4} class="glyph-glow" />}
        <path d={cross} class={`cross${(g.arms?.length ?? 0) > 1 ? " lit" : ""}`} />
        {(g.arms?.length ?? 0) <= 1 && DIRS.map(arm)}
      </g>
    );
  }
  return (
    <g class={`glyph pill${on ? " on" : ""}`}>
      {on && <rect x={x - 3} y={y - size * 0.82 - 3} width={w + 6} height={size * 1.64 + 6} rx={size * 0.82 + 3} class="glyph-glow" />}
      <rect x={x} y={y - size * 0.82} width={w} height={size * 1.64} rx={size * 0.82} />
      <text x={x + w / 2} y={y + size * 0.34} text-anchor="middle" style={{ fontSize: `${size * 0.9}px` }}>
        {g.text}
      </text>
    </g>
  );
}

/** A row of glyphs as an inline SVG, for HTML. */
export function Glyphs({ glyphs, size = 11, on = false }: { glyphs: GlyphSpec[]; size?: number; on?: boolean }) {
  const gap = size * 0.4;
  const pad = on ? 5 : 1;
  let x = pad;
  const items = glyphs.map((g) => {
    const el = <GlyphG g={g} x={x} y={size + pad} size={size} on={on} />;
    x += glyphWidth(g, size) + gap;
    return el;
  });
  const w = Math.max(0, x - gap) + pad;
  return (
    <svg class="glyphs" width={w} height={size * 2 + pad * 2} viewBox={`0 0 ${w} ${size * 2 + pad * 2}`} aria-hidden="true">
      {items}
    </svg>
  );
}

function shortPill(text: string): string {
  return text.length > 18 ? `${text.slice(0, 17)}…` : text;
}

/** Plain words for a glyph, for screen readers and tooltips. */
export function glyphWords(kind: OutKind, control: string, label?: string): string {
  if (kind === "integration" || kind === "keyboard") return label ?? control;
  if (kind === "xbox360") {
    if (FACE[control]) return control.toUpperCase();
    if (control.startsWith("dpad_")) return `D-pad ${control.slice(5)}`;
    return PILL[control] ?? control.toUpperCase();
  }
  return `vJoy ${control}`;
}
