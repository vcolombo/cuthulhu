// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import * as ipc from "../ipc";
import {
  acceptError, acceptResult, controlsFromSpecs, makeDebouncer, svgDataUrl, type PreviewState,
} from "./viewmodel";

const panelStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 100,
};

const dialogStyle: CSSProperties = {
  background: "var(--panel)",
  border: "1px solid var(--border)",
  color: "var(--text)",
  padding: 16,
  width: 640,
  maxHeight: "85vh",
  overflow: "auto",
  display: "flex",
  flexDirection: "column",
  gap: 10,
};

const btn: CSSProperties = {
  background: "var(--panel)",
  color: "var(--text)",
  border: "1px solid var(--border)",
  padding: "4px 10px",
  cursor: "pointer",
};

const previewPane: CSSProperties = {
  flex: 1,
  border: "1px solid var(--border)",
  minHeight: 200,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  fontSize: 12,
  color: "var(--muted)",
  overflow: "hidden",
};

export function TraceDialog({ path, onInsert, onClose }: {
  path: string;
  onInsert: (svg: string) => void;
  onClose: () => void;
}) {
  const [specs, setSpecs] = useState<ipc.TraceControlSpecsDto | null>(null);
  const [controls, setControls] = useState<ipc.TraceControlsDto | null>(null);
  const [preview, setPreview] = useState<PreviewState>({ kind: "idle" });
  const [sourceUrl, setSourceUrl] = useState<string | null>(null);
  const [sourceError, setSourceError] = useState<string | null>(null);
  const latestId = useRef(0);
  const debouncer = useMemo(() => makeDebouncer(300), []);

  useEffect(() => {
    // Guard the thumbnail against out-of-order resolution too: `path` cannot change without
    // unmounting today, but that is a property of the caller, not of this component.
    let ignore = false;
    ipc.loadImagePreview({ path }).then(
      (url) => { if (!ignore) { setSourceUrl(url); setSourceError(null); } },
      // Say why the thumbnail is missing rather than leaving an empty pane. Usually the trace
      // fails for the same reason and reports it, but not always — re-encoding the preview can
      // fail on its own, and then a blank pane beside a successful trace explains nothing.
      (e) => { if (!ignore) { setSourceUrl(null); setSourceError(ipc.ipcErrorMessage(e)); } },
    );
    return () => { ignore = true; };
  }, [path]);

  useEffect(() => {
    let ignore = false;
    ipc.traceControls().then(
      (s) => { if (!ignore) { setSpecs(s); setControls(controlsFromSpecs(s)); } },
      (e) => { if (!ignore) setPreview({ kind: "error", message: ipc.ipcErrorMessage(e) }); },
    );
    return () => { ignore = true; };
  }, []);

  useEffect(() => {
    if (controls === null) return;
    // Both statements must run here, not inside the debounced callback:
    //   - bumping the id retires any in-flight request, so a late response is rejected;
    //   - clearing to `tracing` retires the *displayed* result.
    // Doing only the first still leaves the previous SVG on screen with Insert enabled for the
    // length of the debounce, so a click in that window inserts geometry the sliders no longer
    // describe. The shown result is stale the instant a control moves; say so immediately.
    const id = ++latestId.current;
    setPreview({ kind: "tracing" });
    debouncer.schedule(() => {
      ipc.traceImage({ path, controls }).then(
        (r) => setPreview((prev) => acceptResult(id, latestId.current, r, prev)),
        (e) => setPreview((prev) =>
          acceptError(id, latestId.current, ipc.ipcErrorCode(e), ipc.ipcErrorMessage(e), prev)),
      );
    });
    return () => debouncer.cancel();
  }, [path, controls, debouncer]);

  return (
    <div style={panelStyle}>
      <div role="dialog" aria-modal="true" aria-label="Trace image" style={dialogStyle}>
        <div style={{ display: "flex", alignItems: "center" }}>
          <strong>Trace image</strong>
          <div style={{ flex: 1 }} />
          <button aria-label="Close" style={btn} onClick={onClose}>
            Close
          </button>
        </div>

        <div style={{ display: "flex", gap: 10 }}>
          <div style={previewPane}>
            {sourceUrl ? <img src={sourceUrl} alt="Source" style={{ maxWidth: "100%", maxHeight: 200 }} /> : null}
            {sourceError !== null ? <span style={{ color: "var(--cut)" }}>{sourceError}</span> : null}
          </div>
          <div style={previewPane}>
            {preview.kind === "ready" && <img src={svgDataUrl(preview.svg)} alt="Traced preview" style={{ maxWidth: "100%", maxHeight: 200 }} />}
            {preview.kind === "tracing" && <span>Tracing…</span>}
            {preview.kind === "empty" && <span>{preview.message}</span>}
            {preview.kind === "error" && <span style={{ color: "var(--cut)" }}>{preview.message}</span>}
          </div>
        </div>

        {preview.kind === "ready" ? (
          <div style={{ fontSize: 12, color: "var(--muted)" }}>
            {preview.pathCount} {preview.pathCount === 1 ? "path" : "paths"}
            {preview.downscaled ? ` — large image reduced to ${specs?.maxDim} px for tracing` : ""}
          </div>
        ) : null}

        <div style={{ display: "flex", gap: 16, fontSize: 12 }}>
          <label>
            <input type="radio" checked={controls?.mode === "binary"}
              onChange={() => setControls((c) => c && { ...c, mode: "binary" })} /> Binary
          </label>
          <label>
            <input type="radio" checked={controls?.mode === "color"}
              onChange={() => setControls((c) => c && { ...c, mode: "color" })} /> Color
          </label>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 6, fontSize: 12 }}>
          {specs?.controls.map((s) => {
            const disabled = s.colorOnly && controls?.mode !== "color";
            return (
              <label key={s.name} style={{ display: "flex", alignItems: "center", gap: 8, opacity: disabled ? 0.4 : 1 }}>
                <span style={{ width: 110 }}>{s.label}</span>
                <input type="range" min={s.min} max={s.max} step={s.step} disabled={disabled}
                  value={controls?.[s.name] ?? s.default}
                  onChange={(e) => setControls((c) => c && { ...c, [s.name]: Number(e.target.value) })} />
                <span style={{ width: 32, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>
                  {controls?.[s.name] ?? s.default}
                </span>
              </label>
            );
          })}
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <div style={{ flex: 1 }} />
          <button aria-label="Cancel" style={btn} onClick={onClose}>
            Cancel
          </button>
          <button
            aria-label="Insert"
            style={btn}
            disabled={preview.kind !== "ready"}
            onClick={() => preview.kind === "ready" && onInsert(preview.svg)}
          >
            Insert
          </button>
        </div>
      </div>
    </div>
  );
}
