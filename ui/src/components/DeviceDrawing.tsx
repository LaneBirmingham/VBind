// A simple line drawing of a device with markers where its layout card says
// each control sits. Markers light up live.

import type { Layout } from "../api";

export const SHAPES: Record<string, string[]> = {
  // Flight stick seen from the side: base, column, grip with head.
  stick: [
    "M14 86 Q14 80 20 80 L80 80 Q86 80 86 86 L86 94 Q86 98 82 98 L18 98 Q14 98 14 94 Z",
    "M44 80 L44 70 L56 70 L56 80 Z",
    "M36 70 C32 56 33 34 37 18 C39 8 46 4 54 5 C63 6 67 13 66 22 C64 36 63 56 60 70 Z",
  ],
  throttle: [
    "M10 70 Q10 62 18 62 L82 62 Q90 62 90 70 L90 94 Q90 98 86 98 L14 98 Q10 98 10 94 Z",
    "M30 62 L40 20 Q42 12 50 12 L62 12 Q70 12 68 20 L58 62 Z",
  ],
  gamepad: [
    "M30 22 L70 22 C84 22 90 32 93 48 L97 74 C99 86 88 91 80 83 L70 72 L30 72 L20 83 C12 91 1 86 3 74 L7 48 C10 32 16 22 30 22 Z",
  ],
  wheel: [
    "M50 8 A42 42 0 1 1 49.9 8 Z M50 22 A28 28 0 1 0 50.1 22 Z",
    "M22 54 L78 54 L78 62 L22 62 Z",
  ],
  pedals: [
    "M8 30 Q8 24 14 24 L30 24 Q36 24 36 30 L36 92 Q36 96 32 96 L12 96 Q8 96 8 92 Z",
    "M42 30 Q42 24 48 24 L58 24 Q64 24 64 30 L64 92 Q64 96 60 96 L46 96 Q42 96 42 92 Z",
    "M70 30 Q70 24 76 24 L86 24 Q92 24 92 30 L92 92 Q92 96 88 96 L74 96 Q70 96 70 92 Z",
  ],
  generic: ["M6 10 Q6 4 12 4 L88 4 Q94 4 94 10 L94 90 Q94 96 88 96 L12 96 Q6 96 6 90 Z"],
};

export function DeviceDrawing({ layout, isOn, mapped }: { layout: Layout; isOn: (key: string) => boolean; mapped: (key: string) => boolean }) {
  const placed = Object.entries(layout.controls)
    .filter(([, c]) => c.pos)
    .map(([key, c]) => ({ key, label: c.label, x: c.pos![0], y: c.pos![1], left: c.pos![0] < 50, ly: c.pos![1] }));
  if (!placed.length) return null;
  // Labels sit in a column on each side, nudged apart so they never overlap.
  for (const side of [true, false]) {
    let last = -Infinity;
    for (const p of placed.filter((q) => q.left === side).sort((a, b) => a.y - b.y)) {
      p.ly = Math.max(p.y, last + 7);
      last = p.ly;
    }
  }
  const paths = SHAPES[layout.shape ?? "generic"] ?? SHAPES.generic;
  return (
    <svg class="drawing" viewBox="-62 -4 224 108" role="img" aria-label={`${layout.name} drawing`}>
      {paths.map((d) => (
        <path class="sil" d={d} fill-rule="evenodd" />
      ))}
      {placed.map((p) => {
        const on = isOn(p.key);
        const lx = p.left ? 8 : 92;
        return (
          <g class={on ? "on" : ""}>
            <polyline class="lead" points={`${p.x},${p.y} ${p.left ? lx + 3 : lx - 3},${p.ly} ${lx},${p.ly}`} />
            <circle class={`mk${on ? " on" : ""}${mapped(p.key) ? " mapped" : ""}`} cx={p.x} cy={p.y} r={2.2} />
            <text x={p.left ? lx - 1.5 : lx + 1.5} y={p.ly + 1.5} text-anchor={p.left ? "end" : "start"}>
              {p.label}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
