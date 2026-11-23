// Client for the VBind local API. The page is served by VBind itself, which
// injects the API token as window.VBIND_TOKEN.

declare global {
  interface Window {
    VBIND_TOKEN?: string;
  }
}

const token = (): string => window.VBIND_TOKEN ?? new URLSearchParams(location.search).get("token") ?? "";

export class ApiError extends Error {
  status: number;
  body: any;
  constructor(status: number, body: any) {
    super(typeof body?.error === "string" ? body.error : `request failed (${status})`);
    this.status = status;
    this.body = body;
  }
}

async function call<T = any>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    headers: {
      "X-VBind-Token": token(),
      "X-VBind-Client": "ui",
      ...(body !== undefined ? { "Content-Type": "application/json" } : {}),
    },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  let data: any = text;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    /* plain text */
  }
  if (!res.ok) throw new ApiError(res.status, data);
  return data as T;
}

export const api = {
  get: <T = any>(p: string) => call<T>("GET", p),
  post: <T = any>(p: string, b: unknown = {}) => call<T>("POST", p, b),
  put: <T = any>(p: string, b: unknown) => call<T>("PUT", p, b),
  del: <T = any>(p: string) => call<T>("DELETE", p),
};

export function events(onState: (s: LiveState) => void, onChanged: (what: string) => void): () => void {
  let es: EventSource | null = null;
  let stopped = false;
  let retry = 500;
  const open = () => {
    if (stopped) return;
    es = new EventSource(`/api/events?hz=30&token=${encodeURIComponent(token())}`);
    es.addEventListener("state", (e) => {
      retry = 500;
      try {
        onState(JSON.parse((e as MessageEvent).data));
      } catch {
        /* ignore a bad frame */
      }
    });
    es.addEventListener("changed", (e) => onChanged((e as MessageEvent).data));
    es.onerror = () => {
      es?.close();
      setTimeout(open, retry);
      retry = Math.min(retry * 2, 5000);
      onChanged("reconnect");
    };
  };
  open();
  return () => {
    stopped = true;
    es?.close();
  };
}

// ---------------------------------------------------------------- types

export type Guid = string;

export interface Status {
  version: string;
  port: number;
  polling_hz: number;
  devices: number;
  active_profile: string | null;
  pending: Pending | null;
  outputs: {
    xbox360: { available: boolean; error?: string | null };
    vjoy: { available: boolean; error?: string | null; devices: any[] };
  };
  input_error?: string | null;
  agent: { client: string; seconds_ago: number } | null;
  cli?: string | null;
  data_dir: string;
}

export interface Pending {
  profile: string;
  reason: string;
  roles?: RoleResolution[];
}

export interface Device {
  instance: Guid;
  name: string;
  vid: string | null;
  pid: string | null;
  axes: string[];
  buttons: number;
  hats: number;
  layout: string | null;
  roles: { profile: string; role: string }[];
}

export interface DeviceLive {
  axes: number[];
  hats: (number | null)[];
  buttons: number[];
}

export interface OutputLive {
  name: string;
  kind: "xbox360" | "vjoy";
  xinput_slot?: number;
  vjoy_id?: number;
  error?: string;
  report?: any;
}

export interface LiveState {
  seq: number;
  polling_hz: number;
  devices: Record<Guid, DeviceLive>;
  active: {
    profile: string;
    name: string;
    roles: Record<string, Guid | null>;
    vars: Record<string, number>;
    values: Record<string, number>;
    outputs: OutputLive[];
  } | null;
  pending: Pending | null;
  capture: { prompt: string; elapsed_ms: number } | null;
  simulated: Guid[];
}

export type RoleStatus = "assigned" | "matched" | "ambiguous" | "missing";

export interface RoleResolution {
  role: string;
  status: RoleStatus;
  device?: Guid;
  candidates?: Guid[];
  matching?: Guid[];
  optional: boolean;
  note?: string;
}

export interface ProfileSummary {
  id: string;
  name?: string;
  game?: string | null;
  description?: string;
  outputs?: string[];
  valid: boolean;
  errors: { path: string; message: string; pos?: number }[];
  active: boolean;
  pending?: string | null;
  roles?: RoleResolution[];
  role_labels?: Record<string, string>;
}

export interface GraphRef {
  role: string;
  control: string;
}

export interface Graph {
  profile: string;
  roles: { role: string; label: string; controls: { control: string; label: string; digital: boolean }[] }[];
  vars: { name: string; label?: string; expr: string; depth: number; inputs: GraphRef[]; vars: string[] }[];
  outputs: {
    name: string;
    kind: "xbox360" | "vjoy";
    label: string;
    controls: {
      key: string;
      control: string;
      label: string;
      action?: string;
      kind: "axis" | "trigger" | "button" | "hat_dir";
      expr: string;
      inputs: GraphRef[];
      vars: string[];
    }[];
  }[];
}

export interface GraphResponse {
  graph: Graph;
  devices: { role: string; resolution: RoleResolution; device: { instance: Guid; name: string } | null; layout: string | null }[];
}

export interface Layout {
  vbind_layout: number;
  id: string;
  name: string;
  match: { vid?: string; pid?: string; name?: string }[];
  shape?: "stick" | "throttle" | "gamepad" | "wheel" | "pedals" | "generic";
  controls: Record<string, { label: string; group?: string; pos?: [number, number] }>;
  hidden?: string[];
}

export type Card =
  | { type: "markdown"; id: string; title?: string; markdown: string }
  | { type: "steps"; id: string; title?: string; steps: { text: string; detail?: string; status: "done" | "active" | "todo" | "failed" }[] }
  | { type: "question"; id: string; question: string; options: string[]; allow_text: boolean; answer?: string };

export interface PanelState {
  title?: string;
  cards: Card[];
  highlights: { role?: string; device?: Guid; control: string }[];
  rev: number;
}
