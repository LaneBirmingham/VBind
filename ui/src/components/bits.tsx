import DOMPurify from "dompurify";
import { marked } from "marked";
import type { ComponentChildren } from "preact";
import { useState } from "preact/hooks";

marked.setOptions({ gfm: true, breaks: true });

export function Markdown({ text }: { text: string }) {
  const html = DOMPurify.sanitize(marked.parse(text, { async: false }) as string, { ADD_ATTR: ["target"] });
  return (
    <div
      class="md"
      dangerouslySetInnerHTML={{ __html: html.replace(/<a /g, '<a target="_blank" rel="noreferrer" ') }}
    />
  );
}

/** Horizontal meter. Centered meters fill from the middle for -1..1 values. */
export function Meter({ value, centered, tone }: { value: number; centered?: boolean; tone?: string }) {
  const v = Math.max(-1, Math.min(1, value || 0));
  const style = centered
    ? v >= 0
      ? { left: "50%", width: `${v * 50}%` }
      : { left: `${50 + v * 50}%`, width: `${-v * 50}%` }
    : { left: "0", width: `${Math.max(0, v) * 100}%` };
  return (
    <span class={`meter${centered ? " centered" : ""}`}>
      <span class="fill" style={{ ...style, ...(tone ? { background: tone } : {}) }} />
    </span>
  );
}

export function Lamp({ on }: { on: boolean }) {
  return <span class={`lamp${on ? " on" : ""}`} />;
}

export function Modal({ title, onClose, children, footer, wide }: { title: string; onClose: () => void; children: ComponentChildren; footer?: ComponentChildren; wide?: boolean }) {
  return (
    <div class="scrim" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div class={`modal${wide ? " wide" : ""}`} role="dialog" aria-label={title}>
        <div class="mh">
          <h2>{title}</h2>
          <span class="spacer" />
          <button class="btn ghost small" onClick={onClose} aria-label="Close">
            Close
          </button>
        </div>
        <div class="mb">{children}</div>
        {footer && <div class="mf">{footer}</div>}
      </div>
    </div>
  );
}

export function CopyBox({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div class="codebox">
      {text}
      <button
        class="btn small"
        onClick={async () => {
          try {
            await navigator.clipboard.writeText(text);
          } catch {
            const ta = document.createElement("textarea");
            ta.value = text;
            document.body.appendChild(ta);
            ta.select();
            document.execCommand("copy");
            ta.remove();
          }
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        }}
      >
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

export const TONES = ["var(--tone-0)", "var(--tone-1)", "var(--tone-2)", "var(--tone-3)"];

export function fmt(v: number | undefined, digits = 2): string {
  if (v === undefined || Number.isNaN(v)) return "-";
  const r = Math.round(v * 10 ** digits) / 10 ** digits;
  return (r === 0 ? 0 : r).toFixed(digits);
}

export const XBOX_CONTROLS: [string, string][] = [
  ["lx", "Left stick X"], ["ly", "Left stick Y"], ["rx", "Right stick X"], ["ry", "Right stick Y"],
  ["lt", "Left trigger"], ["rt", "Right trigger"], ["a", "A"], ["b", "B"], ["x", "X"], ["y", "Y"],
  ["lb", "Left bumper"], ["rb", "Right bumper"], ["back", "Back / View"], ["start", "Start / Menu"],
  ["guide", "Guide"], ["ls", "Left stick click"], ["rs", "Right stick click"],
  ["dpad_up", "D-pad up"], ["dpad_down", "D-pad down"], ["dpad_left", "D-pad left"], ["dpad_right", "D-pad right"],
];

export function vjoyControls(): [string, string][] {
  const out: [string, string][] = ["x", "y", "z", "rx", "ry", "rz", "slider1", "slider2"].map((a) => [a, `Axis ${a.toUpperCase()}`]);
  for (let b = 1; b <= 32; b++) out.push([`b${b}`, `Button ${b}`]);
  for (let h = 1; h <= 4; h++) for (const d of ["up", "right", "down", "left"]) out.push([`hat${h}.${d}`, `Hat ${h} ${d}`]);
  return out;
}
