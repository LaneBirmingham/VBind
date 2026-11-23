// Profiles: start or stop them, choose which physical device fills each role,
// import shared profiles, export, edit the JSON and delete.

import { useEffect, useState } from "preact/hooks";
import { api, type ProfileSummary, type RoleResolution } from "../api";
import { activeId, devices, errorText, notify, profiles, refreshGraph, refreshProfiles, selectedProfile, view } from "../state";
import { Modal } from "./bits";

export function ProfilesView() {
  const [editing, setEditing] = useState<string | null>(null);
  const [over, setOver] = useState(false);
  const list = profiles.value;
  return (
    <div class="page">
      <h1>Profiles</h1>
      <p class="muted">
        A profile says which devices to read and how they drive the virtual controllers. Share one by exporting it; whoever imports it
        picks their own devices for each role.
      </p>
      <div class="plist">
        {list.map((p) => (
          <ProfileCard p={p} onEdit={() => setEditing(p.id)} />
        ))}
        <label
          class={`drop${over ? " over" : ""}`}
          onDragOver={(e) => {
            e.preventDefault();
            setOver(true);
          }}
          onDragLeave={() => setOver(false)}
          onDrop={(e) => {
            e.preventDefault();
            setOver(false);
            const files = [...(e.dataTransfer?.files ?? [])];
            files.forEach(importFile);
          }}
        >
          <input
            type="file"
            accept=".json,application/json"
            multiple
            style={{ display: "none" }}
            onChange={(e) => {
              const input = e.target as HTMLInputElement;
              [...(input.files ?? [])].forEach(importFile);
              input.value = "";
            }}
          />
          <b style={{ color: "var(--ink)" }}>Import a profile</b>
          <div class="small">Drop a .vbind.json file here or click to choose one.</div>
        </label>
      </div>
      {editing && <JsonEditor id={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}

export async function importFile(file: File) {
  let data: any;
  try {
    data = JSON.parse(await file.text());
  } catch (e) {
    notify(`${file.name} is not valid JSON: ${(e as Error).message}`, "error");
    return;
  }
  try {
    await api.post("/profiles", data);
    notify(`Imported ${data.name ?? data.id}`);
  } catch (e: any) {
    if (e?.status === 409 && confirm(`A profile called '${data.id}' already exists. Replace it?`)) {
      try {
        await api.post("/profiles?replace=true", data);
        notify(`Replaced ${data.name ?? data.id}`);
      } catch (e2) {
        notify(errorText(e2), "error");
      }
    } else if (e?.status !== 409) {
      notify(`${file.name}: ${errorText(e)}`, "error");
    }
  }
  refreshProfiles();
}

function ProfileCard({ p, onEdit }: { p: ProfileSummary; onEdit: () => void }) {
  const isActive = activeId.value === p.id;
  const [busy, setBusy] = useState(false);
  const run = async (f: () => Promise<unknown>, done?: string) => {
    setBusy(true);
    try {
      await f();
      if (done) notify(done);
    } catch (e) {
      notify(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };
  const exportIt = () =>
    run(async () => {
      const full = await api.get(`/profiles/${encodeURIComponent(p.id)}`);
      const blob = new Blob([JSON.stringify(full, null, 2) + "\n"], { type: "application/json" });
      const a = document.createElement("a");
      a.href = URL.createObjectURL(blob);
      a.download = `${p.id}.vbind.json`;
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 2000);
    });
  return (
    <div class={`pcard${isActive ? " active" : ""}`}>
      <div style={{ minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, flexWrap: "wrap" }}>
          <span class="name">{p.name ?? p.id}</span>
          {isActive && (
            <span class="chip">
              <span class="dot ok" /> Running
            </span>
          )}
          {p.game && <span class="chip">{p.game}</span>}
          {!p.valid && <span class="chip err-text">Has errors</span>}
        </div>
        {p.description && <div class="muted small" style={{ marginTop: 4 }}>{p.description}</div>}
        <div class="small muted mono" style={{ marginTop: 4 }}>
          {p.id}
          {p.outputs?.length ? ` - outputs: ${p.outputs.map((o) => (o === "xbox360" ? "Xbox 360" : "vJoy")).join(", ")}` : ""}
        </div>
        {p.pending && <div class="small warn-text" style={{ marginTop: 4 }}>{p.pending}</div>}
        {!p.valid && (
          <ul class="small err-text" style={{ margin: "6px 0 0", paddingLeft: 18 }}>
            {p.errors.map((e: any) => (
              <li>{typeof e === "string" ? e : `${e.path}: ${e.message}`}</li>
            ))}
          </ul>
        )}
      </div>
      <div class="actions">
        {p.valid &&
          (isActive ? (
            <button class="btn" disabled={busy} onClick={() => run(() => api.post("/deactivate"), "Stopped")}>
              Stop
            </button>
          ) : (
            <button class="btn primary" disabled={busy} onClick={() => run(() => api.post(`/profiles/${encodeURIComponent(p.id)}/activate`), "Started")}>
              Start
            </button>
          ))}
        {p.valid && (
          <button
            class="btn"
            onClick={() => {
              selectedProfile.value = p.id;
              refreshGraph();
              view.value = "mapping";
            }}
          >
            Show mapping
          </button>
        )}
        <button class="btn ghost" onClick={exportIt}>
          Export
        </button>
        <button class="btn ghost" onClick={onEdit}>
          Edit JSON
        </button>
        <button
          class="btn ghost danger"
          disabled={busy}
          onClick={() => confirm(`Delete the profile '${p.name ?? p.id}'? This cannot be undone.`) && run(() => api.del(`/profiles/${encodeURIComponent(p.id)}`), "Deleted")}
        >
          Delete
        </button>
      </div>
      {p.roles && p.roles.length > 0 && (
        <div class="roles">
          <div class="section-title" style={{ marginTop: 6 }}>
            Devices on this PC
          </div>
          {p.roles.map((r) => (
            <RoleRow profile={p.id} r={r} label={p.role_labels?.[r.role] ?? r.role} />
          ))}
        </div>
      )}
    </div>
  );
}

const STATUS_TEXT: Record<RoleResolution["status"], string> = {
  assigned: "Chosen",
  matched: "Found",
  ambiguous: "Several match",
  missing: "Not connected",
};

function RoleRow({ profile, r, label }: { profile: string; r: RoleResolution; label: string }) {
  const [identifying, setIdentifying] = useState(false);
  const all = devices.value;
  const assign = async (guid: string | null) => {
    try {
      await api.put(`/profiles/${encodeURIComponent(profile)}/assignments`, { [r.role]: guid });
      refreshProfiles();
    } catch (e) {
      notify(errorText(e), "error");
    }
  };
  const identify = async () => {
    setIdentifying(true);
    try {
      const res = await api.post<any>("/capture", { prompt: `Press any button on your ${label.toLowerCase()}`, kinds: ["button", "hat"], timeout_s: 20 });
      if (res.timed_out) notify("Nothing was pressed in time.", "error");
      else {
        await assign(res.captured.device);
        notify(`${label} is now ${res.captured.name}`);
      }
    } catch (e) {
      notify(errorText(e), "error");
    } finally {
      setIdentifying(false);
    }
  };
  const candidates = new Set(r.matching ?? r.candidates ?? []);
  const sorted = [...all].sort((a, b) => Number(candidates.has(b.instance)) - Number(candidates.has(a.instance)));
  const cls = r.status === "missing" ? (r.optional ? "muted" : "warn-text") : r.status === "ambiguous" ? "warn-text" : "ok-text";
  return (
    <div class="role-row">
      <span title={`Role '${r.role}' in expressions`}>
        <b>{label}</b>
        {r.optional && <span class="muted small"> (optional)</span>}
      </span>
      <select value={r.device ?? ""} onChange={(e) => assign((e.target as HTMLSelectElement).value || null)}>
        <option value="">{r.status === "missing" ? "Not connected" : "Pick automatically"}</option>
        {sorted.map((d) => (
          <option value={d.instance}>
            {d.name}
            {candidates.has(d.instance) ? "" : " (does not match)"}
          </option>
        ))}
      </select>
      <span style={{ display: "flex", gap: 8, alignItems: "center" }}>
        <span class={`small ${cls}`} title={r.note}>
          {STATUS_TEXT[r.status]}
        </span>
        <button class="btn small" disabled={identifying || !all.length} onClick={identify}>
          {identifying ? "Press a button..." : "Identify"}
        </button>
      </span>
    </div>
  );
}

function JsonEditor({ id, onClose }: { id: string; onClose: () => void }) {
  const [text, setText] = useState<string | null>(null);
  const [problems, setProblems] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api
      .get(`/profiles/${encodeURIComponent(id)}`)
      .then((p) => setText(JSON.stringify(p, null, 2)))
      .catch((e) => {
        notify(errorText(e), "error");
        onClose();
      });
  }, [id]);
  const parse = (): any | null => {
    try {
      return JSON.parse(text ?? "");
    } catch (e) {
      setProblems([`Not valid JSON: ${(e as Error).message}`]);
      return null;
    }
  };
  const validate = async () => {
    const v = parse();
    if (!v) return false;
    const r = await api.post<{ valid: boolean; diagnostics: { path: string; message: string }[] }>("/profiles/validate", v);
    setProblems(r.valid ? [] : r.diagnostics.map((d) => `${d.path}: ${d.message}`));
    return r.valid;
  };
  const save = async () => {
    setBusy(true);
    try {
      if (!(await validate())) return;
      const v = parse();
      if (v.id === id) await api.put(`/profiles/${encodeURIComponent(id)}`, v);
      else await api.post("/profiles", v);
      notify("Profile saved");
      onClose();
    } catch (e) {
      notify(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title={`Edit ${id}`}
      onClose={onClose}
      wide
      footer={
        <>
          {problems?.length === 0 && <span class="ok-text small" style={{ marginRight: "auto", alignSelf: "center" }}>Looks good</span>}
          <button class="btn" onClick={validate} disabled={busy || text === null}>
            Check
          </button>
          <button class="btn primary" onClick={save} disabled={busy || text === null}>
            Save
          </button>
        </>
      }
    >
      <textarea class="text" rows={26} spellcheck={false} value={text ?? "Loading..."} onInput={(e) => setText((e.target as HTMLTextAreaElement).value)} />
      {problems && problems.length > 0 && (
        <ul class="small err-text" style={{ margin: 0, paddingLeft: 18 }}>
          {problems.map((p) => (
            <li>{p}</li>
          ))}
        </ul>
      )}
    </Modal>
  );
}
