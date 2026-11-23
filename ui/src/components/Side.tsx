// Right-hand panel: the inspector for whatever is selected on the board, and
// the agent's panel (plans, steps and questions it posts while it works).

import { signal } from "@preact/signals";
import { useEffect, useMemo, useState } from "preact/hooks";
import { api, type Card, type Graph } from "../api";
import {
  boardProfile,
  deviceName,
  drawerOpen,
  errorText,
  graph,
  integrations,
  live,
  notify,
  outputInputs,
  panel,
  roleInput,
  selection,
  sideTab,
  status,
  varInputs,
} from "../state";
import { fmt, keyboardControls, Lamp, Markdown, Meter, TONES, vjoyControls, XBOX_CONTROLS } from "./bits";
import { shortName } from "./WiringBoard";

/** Expression to start a new mapping with (set by "Map to an output"). */
export const draftExpr = signal("");
export const showConnect = signal(false);

export function Side() {
  if (!drawerOpen.value) return null;
  const tab = sideTab.value;
  const cards = panel.value.cards.length;
  return (
    <aside class="side" aria-label="Side panel">
      <div class="side-head">
        <div class="tabs" role="tablist" style={{ marginLeft: 0 }}>
          <button class="tab" role="tab" aria-selected={tab === "inspect"} onClick={() => (sideTab.value = "inspect")}>
            Inspector
          </button>
          <button class="tab" role="tab" aria-selected={tab === "agent"} onClick={() => (sideTab.value = "agent")}>
            Agent{cards ? ` (${cards})` : ""}
          </button>
        </div>
        <span class="spacer" />
        <button class="btn ghost small" onClick={() => (drawerOpen.value = false)} aria-label="Close panel">
          Close
        </button>
      </div>
      <div class="side-body">{tab === "inspect" ? <Inspector /> : <AgentPanel />}</div>
    </aside>
  );
}

// ---------------------------------------------------------------- inspector

function Inspector() {
  const sel = selection.value;
  const resp = graph.value;
  const profile = boardProfile.value;
  if (!resp || !profile) return <p class="muted">Open a profile on the Mapping tab to inspect it.</p>;
  if (!sel)
    return (
      <p class="muted">
        Click a control, a mixer or an output on the board to see what it does and change it. Your agent can do all of this too.
      </p>
    );
  const g = resp.graph;
  if (sel.startsWith("new:")) return <OutputEditor key={sel + draftExpr.value} profile={profile} g={g} outName={sel.slice(4)} />;
  if (sel.startsWith("in:")) return <InputDetail g={g} sel={sel} />;
  if (sel.startsWith("var:")) return <VarEditor key={sel} profile={profile} g={g} name={sel.slice(4)} />;
  const [outName] = sel.split(".");
  const existing = g.outputs.find((o) => o.name === outName)?.controls.find((c) => c.key === sel);
  if (!existing) return <p class="muted">That mapping no longer exists.</p>;
  return <OutputEditor key={sel + existing.expr + (existing.action ?? "")} profile={profile} g={g} outName={outName} existing={existing} />;
}

type OutControl = Graph["outputs"][number]["controls"][number];

function controlKind(outKind: string, control: string): OutControl["kind"] {
  if (outKind === "keyboard") return ["mouse.x", "mouse.y", "mouse.wheel"].includes(control) ? "axis" : "button";
  if (outKind === "xbox360") {
    if (control === "lt" || control === "rt") return "trigger";
    if (["lx", "ly", "rx", "ry"].includes(control)) return "axis";
    if (control.startsWith("dpad_")) return "hat_dir";
    return "button";
  }
  if (/^hat\d/.test(control)) return "hat_dir";
  if (/^b\d+$/.test(control)) return "button";
  return "axis";
}

const KIND_HELP: Record<OutControl["kind"], string> = {
  axis: "Stick axis: -1 to 1. Up and right are positive.",
  trigger: "Trigger: 0 (released) to 1 (fully pulled).",
  button: "Button: pressed when the value is 0.5 or more.",
  hat_dir: "D-pad direction: on when the value is 0.5 or more.",
};

/** Ask the engine to catch the next control the person presses. */
async function captureExpr(profile: string, kinds: string[], prompt: string): Promise<string | null> {
  try {
    const r = await api.post<any>("/capture", { prompt, kinds, profile, timeout_s: 20 });
    if (r.timed_out) {
      notify("Nothing was pressed in time.", "error");
      return null;
    }
    const role = (r.roles as { profile: string; role: string; expr: string }[]).find((x) => x.profile === profile);
    if (!role) {
      notify(`${r.captured.name} is not one of this profile's devices. Assign it to a role on the Profiles tab first.`, "error");
      return null;
    }
    return role.expr;
  } catch (e) {
    notify(errorText(e), "error");
    return null;
  }
}

function useEval(profile: string, expr: string) {
  const [res, setRes] = useState<{ value?: number; error?: string }>({});
  useEffect(() => {
    if (!expr.trim()) {
      setRes({});
      return;
    }
    let stop = false;
    const run = async () => {
      try {
        const r = await api.post<{ value: number }>("/eval", { expr, profile });
        if (!stop) setRes({ value: r.value });
      } catch (e: any) {
        if (!stop) setRes({ error: e?.body?.pos != null ? `${errorText(e)} (at character ${e.body.pos + 1})` : errorText(e) });
      }
    };
    const t = setTimeout(run, 200);
    const iv = setInterval(run, 250);
    return () => {
      stop = true;
      clearTimeout(t);
      clearInterval(iv);
    };
  }, [profile, expr]);
  return res;
}

function ExprField({ profile, value, onInput, digital }: { profile: string; value: string; onInput: (v: string) => void; digital?: boolean }) {
  const res = useEval(profile, value);
  return (
    <div class="field">
      <label>Expression</label>
      <textarea class="text" rows={3} value={value} spellcheck={false} onInput={(e) => onInput((e.target as HTMLTextAreaElement).value)} />
      <div class="small" style={{ marginTop: 6, display: "flex", gap: 10, alignItems: "center" }}>
        {res.error ? (
          <span class="err-text">{res.error}</span>
        ) : res.value !== undefined ? (
          <>
            <span class="muted">Live:</span>
            {digital ? <Lamp on={res.value >= 0.5} /> : <Meter value={res.value} centered />}
            <span class="mono">{fmt(res.value)}</span>
          </>
        ) : (
          <span class="muted">
            Inputs are <code>role.control</code>, like <code>left.y</code>, <code>right.b1</code> or <code>left.hat1.up</code>.
          </span>
        )}
      </div>
    </div>
  );
}

function OutputEditor({ profile, g, outName, existing }: { profile: string; g: Graph; outName: string; existing?: OutControl }) {
  const out = g.outputs.find((o) => o.name === outName);
  const mapped = new Set(out?.controls.map((c) => c.control) ?? []);
  const integ = out?.kind === "integration" ? integrations.value.find((i) => i.id === out.integration) : undefined;
  const options: [string, string][] =
    out?.kind === "vjoy" ? vjoyControls() : out?.kind === "keyboard" ? keyboardControls() : integ ? (integ.controls ?? []).map((c) => [c.id, c.label]) : XBOX_CONTROLS;
  const free = options.filter(([id]) => !mapped.has(id));
  const [control, setControl] = useState(existing?.control ?? free[0]?.[0] ?? "");
  const [expr, setExpr] = useState(existing?.expr ?? draftExpr.value);
  const [label, setLabel] = useState(existing?.action ?? "");
  const [busy, setBusy] = useState(false);
  if (!out) return <p class="muted">Output '{outName}' is not in this profile.</p>;
  const kind = existing?.kind ?? integ?.controls?.find((c) => c.id === control)?.kind ?? controlKind(out.kind, control);
  const digital = kind === "button" || kind === "hat_dir";
  const key = `${outName}.${control}`;
  const values = live.value?.active?.profile === profile ? live.value.active.values : null;

  const save = async () => {
    setBusy(true);
    try {
      await api.post(`/profiles/${encodeURIComponent(profile)}/map`, { key, expr, label });
      notify(existing ? "Mapping updated" : "Mapping added");
      draftExpr.value = "";
      selection.value = key;
    } catch (e) {
      notify(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };
  const remove = async () => {
    if (!confirm(`Remove the mapping for ${existing!.label}?`)) return;
    try {
      await api.post(`/profiles/${encodeURIComponent(profile)}/map`, { key, expr: null, label: "" });
      notify("Mapping removed");
      selection.value = null;
    } catch (e) {
      notify(errorText(e), "error");
    }
  };
  const capture = async () => {
    const kinds = kind === "axis" ? ["axis"] : kind === "trigger" ? ["axis", "button", "hat"] : ["button", "hat"];
    const got = await captureExpr(profile, kinds, `Press or move what should drive ${out.label}: ${existing?.label ?? control}`);
    if (got) setExpr(got);
  };

  return (
    <>
      <div>
        <div class="section-title">{out.label}</div>
        <h3 style={{ margin: "4px 0 0", font: "600 18px var(--font-display)" }}>
          {existing ? `${shortName(existing.control, existing.label)}${existing.action ? ` - ${existing.action}` : ""}` : "New mapping"}
        </h3>
        {existing && values && (
          <div style={{ display: "flex", gap: 10, alignItems: "center", marginTop: 8 }}>
            {digital ? <Lamp on={(values[key] ?? 0) >= 0.5} /> : <Meter value={values[key] ?? 0} centered={kind === "axis"} tone="var(--accent)" />}
            <span class="mono small">{fmt(values[key])}</span>
          </div>
        )}
      </div>
      {!existing && (
        <div class="field">
          <label>Output control</label>
          <select class="text" value={control} onChange={(e) => setControl((e.target as HTMLSelectElement).value)}>
            {free.map(([id, name]) => (
              <option value={id}>{name}</option>
            ))}
          </select>
        </div>
      )}
      <div class="small muted">{KIND_HELP[kind]}</div>
      <ExprField profile={profile} value={expr} onInput={setExpr} digital={digital} />
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <button class="btn" onClick={capture}>
          Press a control to fill this in
        </button>
      </div>
      <div class="field">
        <label>What it does in the game</label>
        <input class="text" value={label} placeholder="Boost, Fire, Brake..." onInput={(e) => setLabel((e.target as HTMLInputElement).value)} />
      </div>
      {existing && <Sources g={g} refs={outputInputs(g, existing)} vars={existing.vars} />}
      <div style={{ display: "flex", gap: 8 }}>
        <button class="btn primary" disabled={busy || !expr.trim() || !control} onClick={save}>
          {existing ? "Save" : "Add mapping"}
        </button>
        {existing ? (
          <button class="btn danger" onClick={remove}>
            Remove
          </button>
        ) : (
          <button class="btn ghost" onClick={() => (selection.value = null)}>
            Cancel
          </button>
        )}
      </div>
    </>
  );
}

function Sources({ g, refs, vars }: { g: Graph; refs: { role: string; control: string }[]; vars: string[] }) {
  const uniq = [...new Map(refs.map((r) => [`${r.role}.${r.control}`, r])).values()];
  if (!uniq.length && !vars.length) return null;
  return (
    <div class="field">
      <label>Reads from</label>
      <div class="sources">
        {uniq.map((r) => {
          const role = g.roles.find((x) => x.role === r.role);
          const tone = Math.max(0, g.roles.findIndex((x) => x.role === r.role)) % 4;
          const label = role?.controls.find((c) => c.control === r.control)?.label ?? r.control;
          return (
            <button class="chip" style={{ border: 0, cursor: "pointer" }} onClick={() => (selection.value = `in:${r.role}.${r.control}`)}>
              <span class="dot" style={{ background: TONES[tone] }} />
              {role?.label ?? r.role}: {label}
            </button>
          );
        })}
        {vars.map((v) => (
          <button class="chip" style={{ border: 0, cursor: "pointer" }} onClick={() => (selection.value = `var:${v}`)}>
            <span class="dot" style={{ background: "var(--accent-fill)" }} />
            {g.vars.find((x) => x.name === v)?.label ?? v}
          </button>
        ))}
      </div>
    </div>
  );
}

function usedBy(g: Graph, test: (inputs: { role: string; control: string }[], vars: string[]) => boolean) {
  const outs = g.outputs.flatMap((o) => o.controls.filter((c) => test(c.inputs, c.vars)).map((c) => ({ o, c })));
  const vars = g.vars.filter((v) => test(v.inputs, v.vars));
  return { outs, vars };
}

function UsedBy({ g, outs, vars }: { g: Graph; outs: { o: Graph["outputs"][number]; c: OutControl }[]; vars: Graph["vars"] }) {
  return (
    <div class="field">
      <label>Used by</label>
      {!outs.length && !vars.length && <div class="muted small">Nothing uses this yet.</div>}
      <div class="sources">
        {vars.map((v) => (
          <button class="chip" style={{ border: 0, cursor: "pointer" }} onClick={() => (selection.value = `var:${v.name}`)}>
            <span class="dot" style={{ background: "var(--accent-fill)" }} />
            {v.label ?? v.name}
          </button>
        ))}
        {outs.map(({ o, c }) => (
          <button class="chip" style={{ border: 0, cursor: "pointer" }} onClick={() => (selection.value = c.key)}>
            {g.outputs.length > 1 ? `${o.label}: ` : ""}
            {shortName(c.control, c.label)}
            {c.action ? ` (${c.action})` : ""}
          </button>
        ))}
      </div>
    </div>
  );
}

function InputDetail({ g, sel }: { g: Graph; sel: string }) {
  const [role, ...rest] = sel.slice(3).split(".");
  const control = rest.join(".");
  const r = g.roles.find((x) => x.role === role);
  const c = r?.controls.find((x) => x.control === control);
  const dev = graph.value?.devices.find((d) => d.role === role);
  const v = roleInput(role, control);
  const tone = Math.max(0, g.roles.findIndex((x) => x.role === role)) % 4;
  const { outs, vars } = usedBy(g, (inputs) => inputs.some((i) => i.role === role && i.control === control));
  return (
    <>
      <div>
        <div class="section-title">{r?.label ?? role}</div>
        <h3 style={{ margin: "4px 0 0", font: "600 18px var(--font-display)" }}>{c?.label ?? control}</h3>
      </div>
      <div class="kv">
        <span class="k">Device</span>
        <span>{dev?.device ? deviceName(dev.device.instance) || dev.device.name : <span class="warn-text">Not connected</span>}</span>
        <span class="k">In expressions</span>
        <code>
          {role}.{control}
        </code>
        <span class="k">Live</span>
        <span style={{ display: "flex", alignItems: "center", gap: 10 }}>
          {c?.digital ? <Lamp on={Math.abs(v) >= 0.5} /> : <Meter value={v} centered tone={TONES[tone]} />}
          <span class="mono small">{fmt(v)}</span>
        </span>
      </div>
      <UsedBy g={g} outs={outs} vars={vars} />
      <MapTo g={g} expr={`${role}.${control}`} />
    </>
  );
}

function MapTo({ g, expr }: { g: Graph; expr: string }) {
  return (
    <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
      {g.outputs.map((o) => (
        <button
          class="btn"
          onClick={() => {
            draftExpr.value = expr;
            selection.value = `new:${o.name}`;
          }}
        >
          Map to {g.outputs.length > 1 ? o.label : "another output control"}
        </button>
      ))}
    </div>
  );
}

function VarEditor({ profile, g, name }: { profile: string; g: Graph; name: string }) {
  const v = g.vars.find((x) => x.name === name);
  const [expr, setExpr] = useState(v?.expr ?? "");
  const [label, setLabel] = useState(v?.label ?? "");
  const [busy, setBusy] = useState(false);
  const deps = useMemo(() => (v ? varInputs(g, name) : []), [g, name]);
  if (!v) return <p class="muted">That mixer no longer exists.</p>;
  const { outs, vars } = usedBy(g, (_, vs) => vs.includes(name));
  const value = live.value?.active?.profile === profile ? live.value.active.vars[name] : undefined;
  const save = async () => {
    setBusy(true);
    try {
      const p = await api.get<any>(`/profiles/${encodeURIComponent(profile)}`);
      p.vars[name] = label.trim() ? { expr, label: label.trim() } : expr;
      await api.put(`/profiles/${encodeURIComponent(profile)}`, p);
      notify("Mixer updated");
    } catch (e) {
      notify(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <div>
        <div class="section-title">Mixer</div>
        <h3 style={{ margin: "4px 0 0", font: "600 18px var(--font-display)" }}>{v.label ?? v.name}</h3>
        {value !== undefined && (
          <div style={{ display: "flex", gap: 10, alignItems: "center", marginTop: 8 }}>
            <Meter value={value} centered tone="var(--accent)" />
            <span class="mono small">{fmt(value)}</span>
          </div>
        )}
      </div>
      <div class="kv">
        <span class="k">Name</span>
        <code>{v.name}</code>
      </div>
      <ExprField profile={profile} value={expr} onInput={setExpr} />
      <div class="field">
        <label>Label</label>
        <input class="text" value={label} onInput={(e) => setLabel((e.target as HTMLInputElement).value)} />
      </div>
      <Sources g={g} refs={deps} vars={v.vars} />
      <UsedBy g={g} outs={outs} vars={vars} />
      <div style={{ display: "flex", gap: 8 }}>
        <button class="btn primary" disabled={busy || !expr.trim()} onClick={save}>
          Save
        </button>
      </div>
    </>
  );
}

// ---------------------------------------------------------------- agent panel

function ago(s: number) {
  if (s < 10) return "just now";
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  return `${Math.round(s / 3600)} h ago`;
}

function AgentPanel() {
  const p = panel.value;
  const agent = status.value?.agent;
  return (
    <>
      <div class="card" style={{ display: "flex", alignItems: "center", gap: 10 }}>
        <span class={`dot ${agent && agent.seconds_ago < 120 ? "ok" : ""}`} />
        <div style={{ flex: 1, minWidth: 0 }}>
          {agent ? (
            <>
              <div style={{ fontWeight: 600 }}>{agent.client}</div>
              <div class="muted small">Last active {ago(agent.seconds_ago)}</div>
            </>
          ) : (
            <>
              <div style={{ fontWeight: 600 }}>No agent connected yet</div>
              <div class="muted small">Your AI agent posts its plan, steps and questions here.</div>
            </>
          )}
        </div>
        <button class="btn small" onClick={() => (showConnect.value = true)}>
          Connect agent
        </button>
      </div>
      {p.title && <h2 style={{ margin: 0, font: "600 16px var(--font-display)" }}>{p.title}</h2>}
      {!p.cards.length && (
        <p class="muted small">
          Ask your agent something like "set up my flight sticks as an Xbox controller for my game". It finds your devices, asks you to press
          controls, writes the mapping and shows its progress here.
        </p>
      )}
      {p.cards.map((c) => (
        <PanelCard card={c} />
      ))}
      {p.cards.length > 0 && (
        <div>
          <button class="btn ghost small" onClick={() => api.del("/panel").catch((e) => notify(errorText(e), "error"))}>
            Clear panel
          </button>
        </div>
      )}
    </>
  );
}

function PanelCard({ card }: { card: Card }) {
  if (card.type === "markdown")
    return (
      <div class="card">
        {card.title && <h3>{card.title}</h3>}
        <Markdown text={card.markdown} />
      </div>
    );
  if (card.type === "steps")
    return (
      <div class="card">
        {card.title && <h3>{card.title}</h3>}
        <ol class="steps">
          {card.steps.map((s) => (
            <li class={s.status}>
              <span class={`step-mark ${s.status}`}>{s.status === "done" ? "✓" : s.status === "failed" ? "!" : ""}</span>
              <div>
                <div class="txt">{s.text}</div>
                {s.detail && <div class="detail">{s.detail}</div>}
              </div>
            </li>
          ))}
        </ol>
      </div>
    );
  return <Question card={card} />;
}

function Question({ card }: { card: Extract<Card, { type: "question" }> }) {
  const [text, setText] = useState("");
  const answer = async (a: string) => {
    try {
      await api.post("/panel/answer", { id: card.id, answer: a });
    } catch (e) {
      notify(errorText(e), "error");
    }
  };
  return (
    <div class="card" style={{ borderColor: card.answer === undefined ? "var(--accent-fill)" : undefined }}>
      <Markdown text={card.question} />
      {card.answer !== undefined ? (
        <div class="small muted" style={{ marginTop: 6 }}>
          You answered: <b style={{ color: "var(--ink)" }}>{card.answer}</b>
        </div>
      ) : (
        <>
          {card.options.length > 0 && (
            <div class="q-options">
              {card.options.map((o) => (
                <button class="btn small" onClick={() => answer(o)}>
                  {o}
                </button>
              ))}
            </div>
          )}
          {card.allow_text && (
            <form
              style={{ display: "flex", gap: 6, marginTop: 8 }}
              onSubmit={(e) => {
                e.preventDefault();
                if (text.trim()) answer(text.trim());
              }}
            >
              <input class="text" value={text} placeholder="Type an answer" onInput={(e) => setText((e.target as HTMLInputElement).value)} />
              <button class="btn primary small" type="submit">
                Send
              </button>
            </form>
          )}
        </>
      )}
    </div>
  );
}
