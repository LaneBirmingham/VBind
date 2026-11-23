// The picture view of a mapping: each device drawn as itself, with every
// mapped control pointing at the game button it drives. Pictures come from
// layout cards an agent draws; photos are optional and stay on this PC.

import { useMemo, useRef, useState } from "preact/hooks";
import { api, type DrawingView, type Layout } from "../api";
import { deviceName, drawerOpen, errorText, graph, layouts, notify, selection, sideTab } from "../state";
import { CopyBox } from "./bits";
import { buildCallouts, type Callout, calloutLabel, type FigureMode, layoutFor, ViewFigure, viewsFor } from "./Figure";
import { Glyphs } from "./Glyph";
import { Trainer } from "./Trainer";

const MODE_KEY = "vbind.photoMode";

function savedModes(): Record<string, FigureMode> {
  try {
    return JSON.parse(localStorage.getItem(MODE_KEY) ?? "{}");
  } catch {
    return {};
  }
}

export function PictureView() {
  const gr = graph.value;
  const [editing, setEditing] = useState(false);
  const [training, setTraining] = useState(false);
  const callouts = useMemo(() => (gr ? buildCallouts(gr.graph) : new Map<string, Callout[]>()), [gr]);
  if (!gr) return null;
  const roles = gr.graph.roles.filter((r) => callouts.get(r.role)?.length);
  const pick = (co: Callout, what: "callout" | "marker") => {
    selection.value = what === "marker" ? `in:${co.role}.${co.links[0].control}` : co.links[0].out.key;
    drawerOpen.value = true;
    sideTab.value = "inspect";
  };
  const trainable = [...callouts.values()].flat().some((co) => !co.axis);
  return (
    <div class="picture">
      <div class="picture-bar">
        <button class="btn primary" disabled={!trainable} onClick={() => setTraining(true)} title="Practise: VBind shows a game button and you press it on your devices">
          Train
        </button>
        <button class={`btn${editing ? " on" : ""}`} aria-pressed={editing} onClick={() => setEditing(!editing)} title="Drag markers to where the controls really are">
          {editing ? "Done moving" : "Move markers"}
        </button>
        <span class="muted small">
          {editing ? "Drag a marker onto its button. Changes save as you drop them." : "Press anything on your devices to see what it does in the game."}
        </span>
      </div>
      <div class="figures">
        {roles.map((r) => (
          <DeviceFigure
            role={r.role}
            label={r.label}
            device={deviceName(gr.devices.find((d) => d.role === r.role)?.device?.instance)}
            layout={layoutFor(gr, r.role)}
            callouts={callouts.get(r.role) ?? []}
            editing={editing}
            onPick={pick}
          />
        ))}
      </div>
      {training && <Trainer onClose={() => setTraining(false)} />}
    </div>
  );
}

function DeviceFigure({
  role,
  label,
  device,
  layout,
  callouts,
  editing,
  onPick,
}: {
  role: string;
  label: string;
  device: string;
  layout: Layout | null;
  callouts: Callout[];
  editing: boolean;
  onPick: (co: Callout, what: "callout" | "marker") => void;
}) {
  const [modes, setModes] = useState(savedModes);
  const views = viewsFor(layout);
  const placedOn = (pv: (typeof views)[number]) => callouts.filter((co) => layout && layout.controls[co.key] && pv.at(co.key, layout.controls[co.key]));
  const shown = editing ? views : views.filter((pv) => placedOn(pv).length);
  const visible = shown.length ? shown : views.slice(0, 1);
  const unplaced = callouts.filter((co) => !views.some((pv) => layout?.controls[co.key] && pv.at(co.key, layout.controls[co.key])));
  const generic = views.length > 0 && !views[0].real;
  const setMode = (v: DrawingView, m: FigureMode) => {
    const next = { ...modes, [`${layout!.id}/${v.id}`]: m };
    setModes(next);
    try {
      localStorage.setItem(MODE_KEY, JSON.stringify(next));
    } catch {
      /* per-viewer convenience only */
    }
  };
  return (
    <section class="dfig">
      <div class="dfig-head">
        <span class="name">{label}</span>
        {device && <span class="sub">{device}</span>}
      </div>
      {visible.map((pv, i) => {
        const v = pv.view;
        const mode: FigureMode = v.photo ? (modes[`${layout!.id}/${v.id}`] ?? "photo") : "drawing";
        return (
          <div class="dview">
            {(views.length > 1 || v.photo || (editing && pv.real)) && (
              <div class="dview-head">
                <span class="section-title">{v.label}</span>
                {v.photo && (
                  <span class="seg" role="group" aria-label="Show">
                    <button aria-pressed={mode === "drawing"} onClick={() => setMode(v, "drawing")}>
                      Drawing
                    </button>
                    <button aria-pressed={mode === "photo"} onClick={() => setMode(v, "photo")}>
                      Photo
                    </button>
                  </span>
                )}
                {editing && pv.real && layout && <PhotoButton layout={layout} view={v} />}
              </div>
            )}
            <ViewFigure
              layout={layout!}
              pv={pv}
              callouts={placedOn(pv)}
              height={i === 0 ? 440 : 260}
              editing={editing}
              mode={mode}
              onPick={onPick}
            />
          </div>
        );
      })}
      {generic && !editing && <NoPicture device={device || label} layout={layout} hint="This is a generic outline." />}
      {views.length === 0 && <NoPicture device={device || label} layout={layout} />}
      {unplaced.length > 0 && (
        <div class="unplaced">
          <div class="section-title">{views.length ? "Not on the picture yet" : "What each control does"}</div>
          {unplaced.map((co) => (
            <div class="unplaced-row">
              <button class="co inline" onClick={() => onPick(co, "callout")}>
                <span class="co-g">
                  <Glyphs glyphs={co.glyphs} size={12} />
                </span>
                <span class="co-t">
                  <span class="co-a">{co.action}</span>
                  <span class="co-c">{calloutLabel(co, layout)}</span>
                </span>
              </button>
              {editing && layout && views[0]?.real && <PlaceButton layout={layout} view={views[0].view} control={co.key} label={calloutLabel(co, layout)} />}
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function PlaceButton({ layout, view, control, label }: { layout: Layout; view: DrawingView; control: string; label: string }) {
  return (
    <button
      class="btn small"
      onClick={async () => {
        try {
          const body = layout.controls[control] ? [view.size[0] / 2, view.size[1] / 2] : { at: [view.size[0] / 2, view.size[1] / 2], label };
          await api.post(`/layouts/${encodeURIComponent(layout.id)}/place`, { view: view.id, controls: { [control]: body } });
          notify("Placed in the middle. Drag it onto its button.");
        } catch (e) {
          notify(errorText(e), "error");
        }
      }}
    >
      Place
    </button>
  );
}

function readImage(file: File): Promise<{ data: string; width: number; height: number }> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error(`could not read ${file.name}`));
    reader.onload = () => {
      const data = String(reader.result);
      const img = new Image();
      img.onload = () => resolve({ data, width: img.naturalWidth, height: img.naturalHeight });
      img.onerror = () => reject(new Error(`${file.name} is not an image VBind can use (PNG, JPEG or WebP)`));
      img.src = data;
    };
    reader.readAsDataURL(file);
  });
}

function PhotoButton({ layout, view }: { layout: Layout; view: DrawingView }) {
  const input = useRef<HTMLInputElement>(null);
  const base = `/layouts/${encodeURIComponent(layout.id)}/views/${encodeURIComponent(view.id)}/photo`;
  return (
    <>
      <button class="btn small ghost" onClick={() => input.current?.click()} title="Use your own photo of this side of the device. It stays on this PC.">
        {view.photo ? "Replace photo" : "Add photo"}
      </button>
      {view.photo && (
        <button class="btn small ghost danger" onClick={() => api.del(base).catch((e) => notify(errorText(e), "error"))}>
          Remove photo
        </button>
      )}
      <input
        ref={input}
        type="file"
        accept="image/png,image/jpeg,image/webp"
        style={{ display: "none" }}
        onChange={async (e) => {
          const el = e.target as HTMLInputElement;
          const file = el.files?.[0];
          el.value = "";
          if (!file) return;
          try {
            const img = await readImage(file);
            await api.put(base, { data: img.data });
            notify("Photo added. Ask your agent to line it up, or drag the markers onto the buttons.");
          } catch (err) {
            notify(errorText(err), "error");
          }
        }}
      />
    </>
  );
}

/** No picture yet: ask the agent to draw one, or start from a photo. */
function NoPicture({ device, layout, hint }: { device: string; layout: Layout | null; hint?: string }) {
  const input = useRef<HTMLInputElement>(null);
  const prompt = `Draw my ${device} in VBind so I can see my mapping on it: work out what it looks like, confirm which button is which with capture_input, then build it with layout_kit, set_layout_view and place_controls, and check your work with render_layout.`;
  const fromPhoto = async (file: File) => {
    try {
      const img = await readImage(file);
      let id = layout?.id;
      if (!id) {
        notify("This device has no layout card yet. Ask your agent to create one first.", "error");
        return;
      }
      const h = 300;
      const w = Math.round((h * img.width) / Math.max(1, img.height));
      if (!layouts.value.find((l) => l.id === id)?.views?.some((v) => v.id === "photo")) {
        await api.put(`/layouts/${encodeURIComponent(id)}/views/photo`, { label: "Photo", size: [w, h], parts: [] });
      }
      await api.put(`/layouts/${encodeURIComponent(id)}/views/photo/photo`, { data: img.data, fit: [0, 0, w, h] });
      notify("Photo added. Turn on Move markers and place each control on it.");
    } catch (err) {
      notify(errorText(err), "error");
    }
  };
  return (
    <div class="nopic">
      <div>
        <b>{hint ?? `No picture of your ${device} yet.`}</b> Ask your agent to draw it, or use a photo and place the markers yourself.
      </div>
      <CopyBox text={prompt} />
      {layout && (
        <div>
          <button class="btn small" onClick={() => input.current?.click()}>
            Use a photo
          </button>
          <input
            ref={input}
            type="file"
            accept="image/png,image/jpeg,image/webp"
            style={{ display: "none" }}
            onChange={(e) => {
              const el = e.target as HTMLInputElement;
              const f = el.files?.[0];
              el.value = "";
              if (f) fromPhoto(f);
            }}
          />
        </div>
      )}
    </div>
  );
}
