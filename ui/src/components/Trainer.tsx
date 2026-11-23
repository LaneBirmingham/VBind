// Trainer: shows a game button the way a game would ("A - Boost") and waits
// for the person to press the matching control on their devices. Misses come
// back sooner, and a wrong press shows where the right control is.

import { effect } from "@preact/signals";
import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { Graph } from "../api";
import { boardProfile, graph, live, outputInputs, profiles, readControl } from "../state";
import { buildCallouts, calloutLabel, layoutFor, layoutKey, ViewFigure, viewsFor } from "./Figure";
import { glyphFor, Glyphs, glyphWords, type OutKind } from "./Glyph";

type OutCtl = Graph["outputs"][number]["controls"][number];

interface Item {
  key: string;
  out: OutCtl;
  kind: OutKind;
  /** `role.control` ids that drive it. */
  expected: string[];
}

interface Result {
  key: string;
  ok: boolean;
  ms: number;
}

const TIMEOUT_MS = 6000;
const DIGITAL = /^(b\d+|hat\d+\.(up|down|left|right))$/;
const BEST_KEY = "vbind.trainer.best";

function loadBest(profile: string): Record<string, number> {
  try {
    return JSON.parse(localStorage.getItem(BEST_KEY) ?? "{}")[profile] ?? {};
  } catch {
    return {};
  }
}

function saveBest(profile: string, best: Record<string, number>) {
  try {
    const all = JSON.parse(localStorage.getItem(BEST_KEY) ?? "{}");
    all[profile] = best;
    localStorage.setItem(BEST_KEY, JSON.stringify(all));
  } catch {
    /* per-viewer convenience only */
  }
}

function shuffle<T>(xs: T[]): T[] {
  const a = [...xs];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

/** Every digital control currently held on the profile's devices: `role.b3`, `role.hat1.up`. */
function pressedNow(): Set<string> {
  const out = new Set<string>();
  const gr = graph.value;
  const l = live.value;
  if (!gr || !l) return out;
  for (const d of gr.devices) {
    const guid = d.device?.instance;
    const st = guid ? l.devices[guid] : undefined;
    if (!st) continue;
    for (const b of st.buttons) out.add(`${d.role}.b${b}`);
    st.hats.forEach((_, i) => {
      for (const dir of ["up", "down", "left", "right"]) if (readControl(st, `hat${i + 1}.${dir}`) > 0.5) out.add(`${d.role}.hat${i + 1}.${dir}`);
    });
  }
  return out;
}

function trainItems(g: Graph): Item[] {
  const items: Item[] = [];
  for (const o of g.outputs)
    for (const c of o.controls) {
      if (c.kind !== "button" && c.kind !== "hat_dir") continue;
      const expected = [...new Set(outputInputs(g, c).filter((r) => DIGITAL.test(r.control)).map((r) => `${r.role}.${r.control}`))];
      if (expected.length) items.push({ key: c.key, out: c, kind: o.kind, expected });
    }
  return items;
}

type Phase = "intro" | "ask" | "right" | "wrong" | "done";

export function Trainer({ onClose }: { onClose: () => void }) {
  const gr = graph.value!;
  const profile = boardProfile.value ?? gr.graph.profile;
  const all = useMemo(() => trainItems(gr.graph), [gr]);
  const callouts = useMemo(() => buildCallouts(gr.graph), [gr]);
  const [queue, setQueue] = useState<Item[]>([]);
  const [idx, setIdx] = useState(0);
  const [phase, setPhase] = useState<Phase>("intro");
  const [results, setResults] = useState<Result[]>([]);
  const [wrongPress, setWrongPress] = useState<string | null>(null);
  const [best, setBest] = useState(() => loadBest(profile));
  const asked = useRef(0);
  const held = useRef<Set<string>>(new Set());
  const missed = useRef<Set<string>>(new Set());
  const item = queue[idx];

  const start = (items = all) => {
    missed.current = new Set();
    setQueue(shuffle(items));
    setIdx(0);
    setResults([]);
    setWrongPress(null);
    held.current = pressedNow();
    asked.current = performance.now();
    setPhase("ask");
  };
  const next = () => {
    if (idx + 1 >= queue.length) {
      setPhase("done");
      return;
    }
    setIdx(idx + 1);
    setWrongPress(null);
    asked.current = performance.now();
    setPhase("ask");
  };
  const miss = (pressed: string | null) => {
    if (!missed.current.has(item.key)) {
      missed.current.add(item.key);
      setResults((r) => [...r, { key: item.key, ok: false, ms: performance.now() - asked.current }]);
      // Ask it again a little later in this round.
      setQueue((q) => {
        const copy = [...q];
        copy.splice(Math.min(copy.length, idx + 3), 0, item);
        return copy;
      });
    }
    setWrongPress(pressed);
    setPhase("wrong");
  };

  // Watch the devices for new presses.
  useEffect(
    () =>
      effect(() => {
        live.value;
        const now = pressedNow();
        const fresh = [...now].filter((p) => !held.current.has(p));
        held.current = now;
        if (!item || !fresh.length) return;
        if (phase === "ask" && performance.now() - asked.current > 150) {
          const hit = fresh.find((p) => item.expected.includes(p));
          if (hit) {
            const ms = performance.now() - asked.current;
            if (!missed.current.has(item.key)) {
              setResults((r) => [...r, { key: item.key, ok: true, ms }]);
              if (!best[item.key] || ms < best[item.key]) {
                const b = { ...best, [item.key]: Math.round(ms) };
                setBest(b);
                saveBest(profile, b);
              }
            }
            setPhase("right");
          } else miss(fresh[0]);
        } else if (phase === "wrong" && fresh.some((p) => item.expected.includes(p))) {
          setPhase("right");
        }
      }),
    [item, phase, idx, queue, best],
  );

  // Advance after a correct answer; time out unanswered prompts.
  useEffect(() => {
    if (phase === "right") {
      const t = setTimeout(next, 650);
      return () => clearTimeout(t);
    }
    if (phase === "ask") {
      const t = setTimeout(() => miss(null), TIMEOUT_MS);
      return () => clearTimeout(t);
    }
  }, [phase, idx, queue]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      if (e.key === "Enter" && (phase === "intro" || phase === "done")) start();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase]);

  const expectedRef = item?.expected[0];
  const expRole = expectedRef?.split(".")[0];
  const expLayout = expRole ? layoutFor(gr, expRole) : null;
  const expKey = expectedRef ? layoutKey(expectedRef.slice(expRole!.length + 1)) : null;
  const expCallout = expRole ? callouts.get(expRole)?.find((c) => c.key === expKey) : undefined;
  const pressedText = (p: string) => {
    const [role, ...rest] = p.split(".");
    const control = rest.join(".");
    const co = callouts.get(role)?.find((c) => c.key === layoutKey(control));
    const lay = layoutFor(gr, role);
    const name = lay?.controls[layoutKey(control)]?.label ?? control;
    const roleLabel = gr.graph.roles.find((r) => r.role === role)?.label ?? role;
    const what = co ? ` (that is ${co.action})` : " (not mapped)";
    return `${roleLabel}: ${name}${control.includes(".") ? ` ${control.split(".")[1]}` : ""}${what}`;
  };

  const firstTry = results.filter((r) => r.ok);
  const avg = firstTry.length ? firstTry.reduce((a, r) => a + r.ms, 0) / firstTry.length : 0;
  const slowest = [...results].sort((a, b) => (a.ok === b.ok ? b.ms - a.ms : a.ok ? 1 : -1)).slice(0, 3);
  const itemByKey = (k: string) => all.find((i) => i.key === k);

  return (
    <div class="scrim trainer-scrim" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div class="trainer" role="dialog" aria-label="Trainer">
        <div class="trainer-head">
          <b>Trainer</b>
          <span class="muted small">{profiles.value.find((p) => p.id === gr.graph.profile)?.name ?? gr.graph.profile}</span>
          <span class="spacer" />
          {phase !== "intro" && phase !== "done" && (
            <span class="muted small">
              {Math.min(idx + 1, queue.length)} of {queue.length}
            </span>
          )}
          <button class="btn ghost small" onClick={onClose}>
            Close
          </button>
        </div>

        {phase === "intro" && (
          <div class="trainer-body center">
            <h2>Learn where your game buttons are</h2>
            <p class="muted">
              VBind shows a button the way the game does. Press the control on your devices that sends it. {all.length} buttons; misses come back later
              in the round.
            </p>
            <button class="btn primary" onClick={() => start()} disabled={!all.length}>
              Start
            </button>
          </div>
        )}

        {item && (phase === "ask" || phase === "right" || phase === "wrong") && (
          <div class="trainer-body">
            <div class="prompt">
              <div class={`prompt-glyph ${phase}`} key={`${idx}-${item.key}`}>
                <Glyphs glyphs={[glyphFor(item.kind, item.out.control, item.out.label)]} size={34} on={phase === "right"} />
                {phase === "ask" && <span class="timer" style={{ animationDuration: `${TIMEOUT_MS}ms` }} />}
              </div>
              <div>
                <div class="prompt-action">{item.out.action ?? item.out.label}</div>
                <div class="muted">{glyphWords(item.kind, item.out.control, item.out.label)} - press it on your devices</div>
                {phase === "right" && <div class="ok-text prompt-result">Yes: {expCallout ? calloutLabel(expCallout, expLayout) : expKey}</div>}
                {phase === "wrong" && (
                  <div class="prompt-result">
                    <div class="err-text">{wrongPress ? `That was ${pressedText(wrongPress)}.` : "Time is up."}</div>
                    <div>
                      It's <b>{expCallout ? calloutLabel(expCallout, expLayout) : expKey}</b> on your {gr.graph.roles.find((r) => r.role === expRole)?.label.toLowerCase()}. Press it
                      now to carry on.
                    </div>
                    {item.out.hint && <div class="co-h">{item.out.hint}</div>}
                  </div>
                )}
              </div>
            </div>
            {phase === "wrong" && expLayout && expCallout && <Reveal role={expRole!} keyName={expKey!} />}
            {phase === "wrong" && (
              <div style={{ textAlign: "right" }}>
                <button class="btn small ghost" onClick={next}>
                  Skip
                </button>
              </div>
            )}
          </div>
        )}

        {phase === "done" && (
          <div class="trainer-body center">
            <h2>
              {firstTry.length} of {all.length} right first time
            </h2>
            {firstTry.length > 0 && <p class="muted">Average {(avg / 1000).toFixed(2)} s per button.</p>}
            {slowest.length > 0 && (
              <div class="slowest">
                <div class="section-title">Practise these</div>
                {slowest.map((r) => {
                  const it = itemByKey(r.key);
                  if (!it) return null;
                  return (
                    <div class="slow-row">
                      <Glyphs glyphs={[glyphFor(it.kind, it.out.control, it.out.label)]} size={11} />
                      <span>{it.out.action ?? it.out.label}</span>
                      <span class="muted small">{r.ok ? `${(r.ms / 1000).toFixed(2)} s` : "missed"}</span>
                      {best[r.key] && <span class="muted small">best {(best[r.key] / 1000).toFixed(2)} s</span>}
                    </div>
                  );
                })}
              </div>
            )}
            <div style={{ display: "flex", gap: 8, justifyContent: "center" }}>
              <button class="btn primary" onClick={() => start()}>
                Again
              </button>
              {missed.current.size > 0 && (
                <button class="btn" onClick={() => start(all.filter((i) => missed.current.has(i.key)))}>
                  Just the misses
                </button>
              )}
              <button class="btn ghost" onClick={onClose}>
                Close
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

/** A small picture of the device with the right control lit. */
function Reveal({ role, keyName }: { role: string; keyName: string }) {
  const gr = graph.value!;
  const layout = layoutFor(gr, role);
  const co = buildCallouts(gr.graph).get(role)?.find((c) => c.key === keyName);
  if (!layout || !co) return null;
  const pv = viewsFor(layout).find((v) => layout.controls[keyName] && v.at(keyName, layout.controls[keyName]));
  if (!pv) return null;
  const mode = pv.view.photo ? "photo" : "drawing";
  return (
    <div class="reveal">
      <ViewFigure layout={layout} pv={pv} callouts={[co]} height={240} mode={mode} highlight={keyName} showCallouts={false} />
    </div>
  );
}
