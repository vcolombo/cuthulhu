// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import * as ipc from "../ipc";
import {
  acceptError, acceptResult, defaultControls, makeDebouncer, svgDataUrl,
  toOptionsDto, type PreviewState, type TraceControls,
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
  const [controls, setControls] = useState<TraceControls>(defaultControls);
  const [preview, setPreview] = useState<PreviewState>({ kind: "idle" });
  const [sourceUrl, setSourceUrl] = useState<string | null>(null);
  const latestId = useRef(0);
  const debouncer = useMemo(() => makeDebouncer(300), []);

  useEffect(() => {
    ipc.loadImagePreview({ path }).then(setSourceUrl, () => setSourceUrl(null));
  }, [path]);

  useEffect(() => {
    debouncer.schedule(() => {
      const id = ++latestId.current;
      setPreview({ kind: "tracing" });
      ipc.traceImage({ path, opts: toOptionsDto(controls) }).then(
        (r) => setPreview((prev) => acceptResult(id, latestId.current, r, prev)),
        (e) => setPreview((prev) => acceptError(id, latestId.current, String(e), prev)),
      );
    });
    return () => debouncer.cancel();
  }, [path, controls, debouncer]);

  const slider = (label: string, value: number, min: number, max: number, step: number, set: (v: number) => void, disabled = false) => (
    <label style={{ display: "flex", alignItems: "center", gap: 8, opacity: disabled ? 0.4 : 1 }}>
      <span style={{ width: 110 }}>{label}</span>
      <input type="range" min={min} max={max} step={step} value={value} disabled={disabled}
        onChange={(e) => set(Number(e.target.value))} />
      <span style={{ width: 32, textAlign: "right", fontVariantNumeric: "tabular-nums" }}>{value}</span>
    </label>
  );

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
          </div>
          <div style={previewPane}>
            {preview.kind === "ready" && <img src={svgDataUrl(preview.svg)} alt="Traced preview" style={{ maxWidth: "100%", maxHeight: 200 }} />}
            {preview.kind === "tracing" && <span>Tracing…</span>}
            {preview.kind === "empty" && <span>Nothing traced — lower speckle filter or raise detail</span>}
            {preview.kind === "error" && <span style={{ color: "var(--cut)" }}>{preview.message}</span>}
          </div>
        </div>

        {preview.kind === "ready" ? (
          <div style={{ fontSize: 12, color: "var(--muted)" }}>
            {preview.pathCount} paths
            {preview.downscaled ? " — large image reduced to 2048 px for tracing" : ""}
          </div>
        ) : null}

        <div style={{ display: "flex", gap: 16, fontSize: 12 }}>
          <label>
            <input type="radio" checked={controls.mode === "binary"} onChange={() => setControls({ ...controls, mode: "binary" })} /> Binary
          </label>
          <label>
            <input type="radio" checked={controls.mode === "color"} onChange={() => setControls({ ...controls, mode: "color" })} /> Color
          </label>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 6, fontSize: 12 }}>
          {slider("Ignore speckles", controls.speckle, 0, 16, 1, (v) => setControls({ ...controls, speckle: v }))}
          {slider("Smoothing", controls.smoothing, 0, 180, 1, (v) => setControls({ ...controls, smoothing: v }))}
          {slider("Detail", controls.detail, 3.5, 10, 0.5, (v) => setControls({ ...controls, detail: v }))}
          {slider("Colors", controls.colors, 1, 8, 1, (v) => setControls({ ...controls, colors: v }), controls.mode !== "color")}
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
