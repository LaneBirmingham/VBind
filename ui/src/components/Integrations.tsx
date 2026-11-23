// Game integrations: optional per-game add-ons that let a profile drive a
// game directly. VBind ships none; an agent (or the person) imports one, then
// VBind installs its game-side files, starts the game and shows its status.

import { useState } from "preact/hooks";
import { api, type IntegrationSummary } from "../api";
import { errorText, integrations, live, notify, refreshIntegrations } from "../state";
import { fmt, Lamp, Meter, Modal } from "./bits";

function addonWords(i: IntegrationSummary | undefined): { text: string; tone: "ok" | "warn" | "err" | "muted" } {
  const st = i?.live?.status;
  if (!i) return { text: "Integration not installed", tone: "err" };
  if (!st) return { text: "No status from the game", tone: "muted" };
  switch (st.state) {
    case "active":
      return { text: `Add-on active in ${i.game ?? "the game"}`, tone: "ok" };
    case "not_running":
      return { text: `Waiting for ${i.game ?? "the game"} to start`, tone: "muted" };
    case "stale":
      return { text: `${i.game ?? "The game"} has closed`, tone: "muted" };
    default:
      return { text: st.messages?.[0] ?? `Add-on reports ${st.reported ?? "a problem"}`, tone: "err" };
  }
}

/** One line on an output card: is the game-side add-on listening? */
export function AddonStatusText({ id, active, error }: { id?: string; active: boolean; error?: string }) {
  if (error) return <span class="err-text">{error}</span>;
  const i = integrations.value.find((x) => x.id === id);
  const w = addonWords(i);
  const prefix = active ? "" : "Off - ";
  return <span class={w.tone === "ok" && active ? "ok-text" : w.tone === "err" ? "err-text" : "muted"}>{prefix + w.text}</span>;
}

export function IntegrationsSection() {
  const list = integrations.value;
  return (
    <section class="integrations">
      <h2>Game integrations</h2>
      <p class="muted">
        Optional add-ons that let a profile drive a game directly, with true analog input, through a plugin that runs inside the game.
        VBind ships none: your agent adds one for a game that has such a plugin, and you choose when it goes into the game.
      </p>
      {!list.length && <p class="muted small">No integrations yet.</p>}
      <div class="grid">
        {list.map((i) => (
          <IntegrationCard i={i} />
        ))}
      </div>
    </section>
  );
}

function IntegrationCard({ i }: { i: IntegrationSummary }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmInstall, setConfirmInstall] = useState(false);
  const ins = i.install;
  const st = addonWords(i);
  const out = live.value?.active?.outputs.find((o) => o.integration === i.id);
  const values = (out?.report ?? {}) as Record<string, number>;

  const run = async (what: "install" | "uninstall" | "launch") => {
    setBusy(what);
    try {
      await api.post(`/integrations/${encodeURIComponent(i.id)}/${what}`);
      notify(what === "install" ? "Add-on files installed" : what === "uninstall" ? "Add-on files removed" : "Asked Steam to start the game");
    } catch (e) {
      notify(errorText(e), "error");
    } finally {
      setBusy(null);
      refreshIntegrations();
    }
  };

  if (!i.valid) {
    return (
      <div class="dcard">
        <div class="top">
          <div class="name">{i.id}</div>
        </div>
        <div class="err-text small">
          {i.errors.map((e) => (
            <div>
              {e.path}: {e.message}
            </div>
          ))}
        </div>
      </div>
    );
  }

  const g = ins.game;
  const needs = ins.needs_install;
  const outdated = ins.installed && ins.current === false;
  const groups = new Map<string, NonNullable<IntegrationSummary["controls"]>>();
  for (const c of i.controls ?? []) {
    const k = c.group ?? "";
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k)!.push(c);
  }
  const readouts = (i.live?.readouts ?? []).filter((r) => r.value != null);

  return (
    <div class="dcard integ">
      <div class="top">
        <div style={{ flex: 1, minWidth: 0 }}>
          <div class="name">{i.name}</div>
          <div class="meta">
            {i.id}
            {i.source?.version ? ` - add-on ${i.source.version}` : ""}
            {i.source?.license ? ` - ${i.source.license}` : ""}
          </div>
        </div>
        {i.driving && <span class="chip">In use</span>}
      </div>

      <div class="integ-status">
        <Lamp on={st.tone === "ok"} />
        <span class={st.tone === "ok" ? "ok-text" : st.tone === "err" ? "err-text" : "muted"}>{st.text}</span>
      </div>

      <dl class="integ-facts">
        <dt>Game</dt>
        <dd>
          {g ? (
            <>
              {g.name ?? i.game} <span class="muted">build {g.build_id ?? "?"}</span>
              {g.updating && <span class="warn-text"> - Steam is updating it</span>}
              {ins.tested_build === false && <span class="warn-text"> - not a build the add-on was tested on</span>}
            </>
          ) : (
            <span class="warn-text">{i.game ?? "The game"} was not found through Steam</span>
          )}
        </dd>
        {needs && (
          <>
            <dt>Add-on files</dt>
            <dd>
              {!ins.installed && <span class="muted">Not installed</span>}
              {ins.installed && !outdated && <span class="ok-text">Installed in the game folder</span>}
              {outdated && <span class="warn-text">The game folder changed (a game update?). Install again.</span>}
            </dd>
          </>
        )}
        <dt>Profiles</dt>
        <dd>{i.used_by.length ? i.used_by.join(", ") : <span class="muted">None use it yet</span>}</dd>
      </dl>

      {readouts.length > 0 && (
        <div class="readouts">
          {readouts.map((r) => (
            <div class="readout">
              <span class="rl">{r.label}</span>
              <span class="rv">{typeof r.value === "string" ? r.value : JSON.stringify(r.value)}</span>
            </div>
          ))}
        </div>
      )}

      {i.driving && (
        <div>
          <div class="section-title" style={{ marginBottom: 6 }}>
            Sending to the game
          </div>
          {[...groups.entries()].map(([group, cs]) => (
            <div class="integ-group">
              {group && <div class="small muted">{group}</div>}
              {cs.some((c) => c.kind !== "button") && (
                <div class="axes">
                  {cs
                    .filter((c) => c.kind !== "button")
                    .map((c) => {
                      const v = values[c.id] ?? 0;
                      return (
                        <>
                          <span>{c.label}</span>
                          <Meter value={v} centered={c.kind === "axis"} />
                          <span class="mono small" style={{ textAlign: "right" }}>
                            {fmt(v)}
                          </span>
                        </>
                      );
                    })}
                </div>
              )}
              {cs.some((c) => c.kind === "button") && (
                <div class="integ-buttons">
                  {cs
                    .filter((c) => c.kind === "button")
                    .map((c) => (
                      <span class={`bcell wide${(values[c.id] ?? 0) >= 0.5 ? " on" : ""}`} title={c.description ?? c.label}>
                        {c.label}
                      </span>
                    ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      <div class="integ-actions">
        {needs && (
          <button class="btn small" disabled={!!busy || !g || g.updating} onClick={() => setConfirmInstall(true)}>
            {busy === "install" ? "Installing..." : ins.installed ? "Install again" : "Install add-on"}
          </button>
        )}
        {i.has_launch && (
          <button class="btn small primary" disabled={!!busy || !g || (needs && !ins.installed)} onClick={() => run("launch")}>
            {busy === "launch" ? "Starting..." : "Start game"}
          </button>
        )}
        {needs && ins.installed && (
          <button class="btn small ghost" disabled={!!busy} onClick={() => run("uninstall")}>
            {busy === "uninstall" ? "Removing..." : "Uninstall"}
          </button>
        )}
        {i.source?.url && (
          <a class="btn small ghost" href={i.source.url} target="_blank" rel="noreferrer">
            Source
          </a>
        )}
      </div>

      {confirmInstall && (
        <Modal
          title={`Install the ${i.name} add-on?`}
          onClose={() => setConfirmInstall(false)}
          footer={
            <>
              <button class="btn ghost" onClick={() => setConfirmInstall(false)}>
                Cancel
              </button>
              <button
                class="btn primary"
                onClick={() => {
                  setConfirmInstall(false);
                  run("install");
                }}
              >
                Install
              </button>
            </>
          }
        >
          <p>
            VBind copies the add-on's files into {g?.dir ?? "the game folder"}. Any file it replaces is kept and put back when you
            uninstall. Close the game first.
          </p>
          {i.source?.notes && <p class="muted small">{i.source.notes}</p>}
          <p class="muted small">
            Add-ons that hook a game can break after a game update, and may not be allowed on every multiplayer server. Use them where the
            game's rules allow.
          </p>
        </Modal>
      )}
    </div>
  );
}
