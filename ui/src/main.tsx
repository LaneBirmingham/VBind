import { render } from "preact";
import { useRef, useState } from "preact/hooks";
import { api } from "./api";
import { ConnectAgent } from "./components/ConnectAgent";
import { DevicesView } from "./components/DevicesView";
import { PictureView } from "./components/PictureView";
import { importFile, ProfilesView } from "./components/ProfilesView";
import { draftExpr, showConnect, Side } from "./components/Side";
import { WiringBoard } from "./components/WiringBoard";
import {
  activeId,
  boardProfile,
  connected,
  devices,
  drawerOpen,
  errorText,
  graph,
  graphError,
  live,
  notify,
  profiles,
  refreshGraph,
  selectedProfile,
  selection,
  sideTab,
  start,
  status,
  toast,
  view,
  type View,
} from "./state";
import "./styles.css";

const VIGEM_URL = "https://github.com/nefarius/ViGEmBus/releases";
const VJOY_URL = "https://github.com/BrunnerInnovation/vJoy/releases";

function TopBar() {
  const v = view.value;
  const st = status.value;
  const board = boardProfile.value;
  const running = !!board && activeId.value === board;
  const valid = profiles.value.filter((p) => p.valid);
  const agent = st?.agent;
  const agentLive = !!agent && agent.seconds_ago < 120;
  const tabs: [View, string][] = [
    ["mapping", "Mapping"],
    ["devices", `Devices${devices.value.length ? ` (${devices.value.length})` : ""}`],
    ["profiles", "Profiles"],
  ];
  const toggle = async () => {
    if (!board) return;
    try {
      if (running) {
        await api.post("/deactivate");
        notify("Stopped. The virtual controller is unplugged.");
      } else {
        await api.post(`/profiles/${encodeURIComponent(board)}/activate`);
        notify("Running");
      }
    } catch (e) {
      notify(errorText(e), "error");
    }
  };
  return (
    <header class="topbar">
      <div class="brand">
        <img src="/icon.svg" alt="" />
        VBind
      </div>
      <nav class="tabs" role="tablist">
        {tabs.map(([id, label]) => (
          <button class="tab" role="tab" aria-selected={v === id} onClick={() => (view.value = id)}>
            {label}
          </button>
        ))}
      </nav>
      <span class="spacer" />
      {valid.length > 0 && (
        <>
          <select
            class="profile-select"
            aria-label="Profile"
            value={board ?? ""}
            onChange={(e) => {
              selectedProfile.value = (e.target as HTMLSelectElement).value;
              selection.value = null;
              refreshGraph();
              view.value = "mapping";
            }}
          >
            {valid.map((p) => (
              <option value={p.id}>
                {p.name ?? p.id}
                {p.active ? " (running)" : ""}
              </option>
            ))}
          </select>
          <button class={`btn${running ? "" : " primary"}`} onClick={toggle} title={running ? "Unplug the virtual controllers" : "Plug in the virtual controllers"}>
            {running ? "Stop" : "Start"}
          </button>
        </>
      )}
      {st && (
        <span class="chip" title={st.outputs.xbox360.error ?? "ViGEmBus driver found"}>
          <span class={`dot ${st.outputs.xbox360.available ? "ok" : "err"}`} /> Xbox
        </span>
      )}
      {st && (st.outputs.vjoy.available || profiles.value.some((p) => p.outputs?.includes("vjoy"))) && (
        <span class="chip" title={st.outputs.vjoy.error ?? "vJoy driver found"}>
          <span class={`dot ${st.outputs.vjoy.available ? "ok" : "warn"}`} /> vJoy
        </span>
      )}
      <button
        class="btn ghost"
        onClick={() => {
          if (!agent) showConnect.value = true;
          else {
            drawerOpen.value = !(drawerOpen.value && sideTab.value === "agent");
            sideTab.value = "agent";
          }
        }}
        title={agent ? `${agent.client}, last active ${agent.seconds_ago}s ago` : "Connect an AI agent"}
      >
        <span class={`dot ${agentLive ? "ok" : ""}`} />
        {agent ? (agentLive ? "Agent connected" : "Agent idle") : "Connect agent"}
      </button>
    </header>
  );
}

function Banners() {
  const l = live.value;
  const st = status.value;
  const pending = l?.pending ?? st?.pending;
  const pendingName = profiles.value.find((p) => p.id === pending?.profile)?.name ?? pending?.profile;
  const usesXbox = profiles.value.some((p) => (p.active || p.id === boardProfile.value) && p.outputs?.includes("xbox360"));
  const usesVjoy = profiles.value.some((p) => (p.active || p.id === boardProfile.value) && p.outputs?.includes("vjoy"));
  return (
    <>
      {l?.capture && (
        <div class="banner capture" role="status">
          <span class="pulse" />
          <div style={{ flex: 1 }}>
            <div class="title">{l.capture.prompt}</div>
            <div class="small muted">VBind is listening for the next button, hat or stick you move.</div>
          </div>
          <button class="btn" onClick={() => api.del("/capture")}>
            Cancel
          </button>
        </div>
      )}
      {pending && !l?.active && (
        <div class="banner warn" role="status">
          <span class="dot warn" />
          <div style={{ flex: 1 }}>
            <div class="title">{pendingName} starts when its devices are ready</div>
            <div class="small muted">{pending.reason}</div>
          </div>
          <button class="btn" onClick={() => (view.value = "profiles")}>
            Choose devices
          </button>
        </div>
      )}
      {l && l.simulated.length > 0 && (
        <div class="banner" role="status">
          <span class="dot slate" />
          <div style={{ flex: 1 }}>
            <div class="title">Your agent is simulating input</div>
            <div class="small muted">Readings from {l.simulated.length} device(s) are being overridden for a test.</div>
          </div>
          <button class="btn" onClick={() => api.del("/simulate")}>
            Stop test
          </button>
        </div>
      )}
      {st && usesXbox && !st.outputs.xbox360.available && (
        <div class="banner warn">
          <span class="dot err" />
          <div style={{ flex: 1 }}>
            <div class="title">The ViGEmBus driver is needed for virtual Xbox controllers</div>
            <div class="small muted">{st.outputs.xbox360.error}</div>
          </div>
          <a class="btn" href={VIGEM_URL} target="_blank" rel="noreferrer">
            Get ViGEmBus
          </a>
        </div>
      )}
      {st && usesVjoy && !st.outputs.vjoy.available && (
        <div class="banner warn">
          <span class="dot warn" />
          <div style={{ flex: 1 }}>
            <div class="title">The vJoy driver is needed for virtual flight sticks</div>
            <div class="small muted">{st.outputs.vjoy.error}</div>
          </div>
          <a class="btn" href={VJOY_URL} target="_blank" rel="noreferrer">
            Get vJoy
          </a>
        </div>
      )}
    </>
  );
}

const MAPPING_MODE_KEY = "vbind.mappingMode";

function savedMappingMode(): "wiring" | "picture" {
  try {
    return localStorage.getItem(MAPPING_MODE_KEY) === "picture" ? "picture" : "wiring";
  } catch {
    return "wiring";
  }
}

function MappingView() {
  const file = useRef<HTMLInputElement>(null);
  const [mode, setModeState] = useState(savedMappingMode);
  const setMode = (m: "wiring" | "picture") => {
    setModeState(m);
    try {
      localStorage.setItem(MAPPING_MODE_KEY, m);
    } catch {
      /* per-viewer convenience only */
    }
  };
  const resp = graph.value;
  const board = boardProfile.value;
  const summary = profiles.value.find((p) => p.id === board);
  if (!profiles.value.length)
    return (
      <div class="empty">
        <h1>Nothing mapped yet</h1>
        <p>
          VBind starts empty. Connect your AI agent and ask it to set up your controllers, or import a profile someone shared with you.
        </p>
        <div class="actions">
          <button class="btn primary" onClick={() => (showConnect.value = true)}>
            Connect your agent
          </button>
          <button class="btn" onClick={() => file.current?.click()}>
            Import a profile
          </button>
          <button class="btn ghost" onClick={() => (view.value = "devices")}>
            See my devices
          </button>
        </div>
        <input
          ref={file}
          type="file"
          accept=".json,application/json"
          style={{ display: "none" }}
          onChange={(e) => {
            const input = e.target as HTMLInputElement;
            [...(input.files ?? [])].forEach(importFile);
            input.value = "";
          }}
        />
      </div>
    );
  if (graphError.value)
    return (
      <div class="page">
        <h1>{summary?.name ?? board}</h1>
        <div class="banner warn" style={{ margin: "12px 0" }}>
          <span class="dot err" />
          <div style={{ flex: 1, whiteSpace: "pre-wrap" }}>{graphError.value}</div>
          <button class="btn" onClick={() => (view.value = "profiles")}>
            Open profiles
          </button>
        </div>
      </div>
    );
  if (!resp) return <div class="page muted">Loading...</div>;
  return (
    <div class="board-wrap">
      <div class="board-head">
        <h1>{summary?.name ?? resp.graph.profile}</h1>
        {summary?.game && <span class="chip">{summary.game}</span>}
        <span class="seg big" role="group" aria-label="Show the mapping as">
          <button aria-pressed={mode === "wiring"} onClick={() => setMode("wiring")} title="How the profile works: inputs, mixers and outputs">
            Wiring
          </button>
          <button aria-pressed={mode === "picture"} onClick={() => setMode("picture")} title="Where things are: your devices, with each control's game button">
            Picture
          </button>
        </span>
      </div>
      {summary?.description && <p class="board-desc">{summary.description}</p>}
      {mode === "picture" ? (
        <PictureView />
      ) : (
        <WiringBoard
          onAddMapping={(output) => {
            draftExpr.value = "";
            selection.value = `new:${output}`;
            drawerOpen.value = true;
            sideTab.value = "inspect";
          }}
        />
      )}
    </div>
  );
}

function App() {
  const v = view.value;
  const t = toast.value;
  return (
    <div class="app">
      <TopBar />
      <div class="main">
        <main class="content">
          <Banners />
          {v === "mapping" ? <MappingView /> : v === "devices" ? <DevicesView devices={devices.value} /> : <ProfilesView />}
        </main>
        <Side />
      </div>
      {!connected.value && (
        <div class="offline">
          <div>
            <h2 style={{ font: "600 20px var(--font-display)", margin: "0 0 6px" }}>Reconnecting to VBind...</h2>
            <p class="muted">The VBind engine stopped responding. It reconnects on its own when it is back.</p>
          </div>
        </div>
      )}
      {showConnect.value && <ConnectAgent onClose={() => (showConnect.value = false)} />}
      {t && <div class={`toast ${t.kind}`} role="status">{t.text}</div>}
    </div>
  );
}

start();
render(<App />, document.getElementById("app")!);
