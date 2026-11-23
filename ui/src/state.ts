// App state as signals, kept fresh from the API and its event stream.

import { computed, signal } from "@preact/signals";
import {
  api,
  events,
  type Device,
  type Drawing,
  type Graph,
  type GraphRef,
  type IntegrationSummary,
  type GraphResponse,
  type Layout,
  type LiveState,
  type PanelState,
  type ProfileSummary,
  type Status,
} from "./api";

export type View = "mapping" | "devices" | "profiles";

export const view = signal<View>("mapping");
export const status = signal<Status | null>(null);
export const devices = signal<Device[]>([]);
export const profiles = signal<ProfileSummary[]>([]);
export const layouts = signal<Layout[]>([]);
/** Resolved pictures by layout id, for layouts that have views. */
export const drawings = signal<Record<string, Drawing>>({});
export const live = signal<LiveState | null>(null);
/** Installed game integrations with their add-on status. */
export const integrations = signal<IntegrationSummary[]>([]);
export const panel = signal<PanelState>({ cards: [], highlights: [], rev: 0 });
export const connected = signal(true);
/** Profile shown on the mapping board: the user's pick, else the active one. */
export const selectedProfile = signal<string | null>(null);
export const graph = signal<GraphResponse | null>(null);
export const graphError = signal<string | null>(null);
export const drawerOpen = signal(false);
export const sideTab = signal<"inspect" | "agent">("agent");
export const toast = signal<{ text: string; kind: "ok" | "error" } | null>(null);
/** What the inspector shows: an output key, `var:<name>` or `in:<role>.<control>`. */
export const selection = signal<string | null>(null);

export const activeId = computed(() => live.value?.active?.profile ?? status.value?.active_profile ?? null);
export const boardProfile = computed(() => {
  const pick = selectedProfile.value;
  if (pick && profiles.value.some((p) => p.id === pick)) return pick;
  return activeId.value ?? profiles.value.find((p) => p.valid)?.id ?? null;
});

let toastTimer: number | undefined;
export function notify(text: string, kind: "ok" | "error" = "ok") {
  toast.value = { text, kind };
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (toast.value = null), kind === "error" ? 7000 : 3000);
}

export function errorText(e: unknown): string {
  const anyE = e as any;
  const diags = anyE?.body?.diagnostics as { path: string; message: string }[] | undefined;
  if (diags?.length) return diags.map((d) => `${d.path}: ${d.message}`).join("\n");
  return anyE?.message ?? String(e);
}

export async function refreshStatus() {
  try {
    status.value = await api.get<Status>("/status");
    connected.value = true;
  } catch {
    connected.value = false;
  }
}
export async function refreshDevices() {
  devices.value = await api.get<Device[]>("/devices").catch(() => devices.value);
}
export async function refreshProfiles() {
  profiles.value = await api.get<ProfileSummary[]>("/profiles").catch(() => profiles.value);
}
export async function refreshLayouts() {
  layouts.value = await api.get<Layout[]>("/layouts").catch(() => layouts.value);
  const next: Record<string, Drawing> = {};
  await Promise.all(
    layouts.value
      .filter((l) => l.views?.length)
      .map(async (l) => {
        const d = await api.get<Drawing>(`/layouts/${encodeURIComponent(l.id)}/drawing`).catch(() => drawings.value[l.id]);
        if (d) next[l.id] = d;
      }),
  );
  drawings.value = next;
}
export async function refreshIntegrations() {
  integrations.value = await api.get<IntegrationSummary[]>("/integrations").catch(() => integrations.value);
}
export async function refreshPanel() {
  const p = await api.get<PanelState>("/panel").catch(() => null);
  if (!p) return;
  const hadCards = panel.value.cards.length;
  panel.value = p;
  if (p.cards.length > hadCards) {
    drawerOpen.value = true;
    sideTab.value = "agent";
  }
}
export async function refreshGraph() {
  const id = boardProfile.value;
  if (!id) {
    graph.value = null;
    graphError.value = null;
    return;
  }
  try {
    graph.value = await api.get<GraphResponse>(`/profiles/${encodeURIComponent(id)}/graph`);
    graphError.value = null;
  } catch (e) {
    graph.value = null;
    graphError.value = errorText(e);
  }
}

let lastBoard: string | null = null;
export function start() {
  const all = async () => {
    await Promise.all([refreshStatus(), refreshDevices(), refreshProfiles(), refreshLayouts(), refreshPanel(), refreshIntegrations()]);
    await refreshGraph();
  };
  all();
  events(
    (s) => {
      live.value = s;
      connected.value = true;
      if (boardProfile.value !== lastBoard) {
        lastBoard = boardProfile.value;
        refreshGraph();
      }
    },
    (what) => {
      switch (what) {
        case "devices":
          refreshDevices();
          refreshProfiles();
          refreshGraph();
          refreshStatus();
          break;
        case "profiles":
        case "active":
          refreshProfiles();
          refreshStatus();
          refreshGraph();
          refreshDevices();
          break;
        case "layouts":
          refreshLayouts();
          refreshGraph();
          refreshDevices();
          break;
        case "panel":
          refreshPanel();
          break;
        case "integrations":
          refreshIntegrations();
          refreshProfiles();
          refreshGraph();
          break;
        case "reconnect":
          connected.value = false;
          setTimeout(all, 800);
          break;
      }
    },
  );
  setInterval(refreshStatus, 4000);
  // The add-on's status lives in the game, so ask now and then.
  setInterval(() => {
    if (integrations.value.length && !document.hidden) refreshIntegrations();
  }, 2500);
}

// ---------------------------------------------------------------- helpers

/** Live value of an input control on a role in the active profile. */
export function inputValue(role: string, control: string): number {
  const l = live.value;
  const guid = l?.active?.roles?.[role];
  if (!l || !guid) return 0;
  return readControl(l.devices[guid], control);
}

export function readControl(d: LiveState["devices"][string] | undefined, control: string): number {
  if (!d) return 0;
  const axes = ["x", "y", "z", "rx", "ry", "rz", "slider1", "slider2"];
  const ai = axes.indexOf(control);
  if (ai >= 0) return d.axes[ai] ?? 0;
  const b = /^b(\d+)$/.exec(control);
  if (b) return d.buttons.includes(Number(b[1])) ? 1 : 0;
  const h = /^hat(\d+)\.(\w+)$/.exec(control);
  if (h) {
    const angle = d.hats[Number(h[1]) - 1];
    if (angle == null) return 0;
    const dirs: Record<string, number> = { up: 0, up_right: 4500, right: 9000, down_right: 13500, down: 18000, down_left: 22500, left: 27000, up_left: 31500 };
    if (h[2] === "x") return near(angle, 9000, 6750) ? 1 : near(angle, 27000, 6750) ? -1 : 0;
    if (h[2] === "y") return near(angle, 0, 6750) ? 1 : near(angle, 18000, 6750) ? -1 : 0;
    const t = dirs[h[2]];
    return t !== undefined && near(angle, t, 2300) ? 1 : 0;
  }
  return 0;
}

function near(a: number, b: number, tol: number) {
  let d = Math.abs(a - b) % 36000;
  if (d > 18000) d = 36000 - d;
  return d <= tol;
}

export function deviceName(guid: string | null | undefined): string {
  if (!guid) return "";
  return devices.value.find((d) => d.instance === guid)?.name ?? guid;
}

/** Colour slot for a role, by its position in the profile. */
export function roleTone(role: string): number {
  const roles = graph.value?.graph.roles.map((r) => r.role) ?? [];
  const i = roles.indexOf(role);
  return i < 0 ? 0 : i % 4;
}

/** Every physical input a variable depends on, following other variables. */
export function varInputs(g: Graph, name: string, seen = new Set<string>()): GraphRef[] {
  if (seen.has(name)) return [];
  seen.add(name);
  const v = g.vars.find((x) => x.name === name);
  if (!v) return [];
  return [...v.inputs, ...v.vars.flatMap((d) => varInputs(g, d, seen))];
}

/** Every physical input an output control reads, directly or through variables. */
export function outputInputs(g: Graph, c: Graph["outputs"][number]["controls"][number]): GraphRef[] {
  const seen = new Set<string>();
  return [...c.inputs, ...c.vars.flatMap((d) => varInputs(g, d, seen))];
}

/** Live value of an input on a role, for any profile whose role resolves to a device. */
export function roleInput(role: string, control: string): number {
  const gr = graph.value;
  const guid = gr?.devices.find((d) => d.role === role)?.device?.instance ?? live.value?.active?.roles?.[role];
  if (!guid) return 0;
  return readControl(live.value?.devices[guid], control);
}
