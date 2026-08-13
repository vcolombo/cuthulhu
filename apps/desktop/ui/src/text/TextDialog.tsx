// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useState, type CSSProperties } from "react";
import * as ipc from "../ipc";
import { canInsert, fontsLoaded, selectFamily, type FontListState } from "./viewmodel";

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
  width: 360,
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

export function TextDialog({ onInsert, onClose }: {
  onInsert: (family: string) => void;
  onClose: () => void;
}) {
  const [fonts, setFonts] = useState<FontListState>({ kind: "loading" });

  useEffect(() => {
    let ignore = false;
    ipc.listFonts().then(
      (families) => { if (!ignore) setFonts(fontsLoaded(families)); },
      (e) => { if (!ignore) setFonts({ kind: "error", message: ipc.ipcErrorMessage(e) }); },
    );
    return () => { ignore = true; };
  }, []);

  return (
    <div style={panelStyle}>
      <div role="dialog" aria-modal="true" aria-label="Add text" style={dialogStyle}>
        <div style={{ display: "flex", alignItems: "center" }}>
          <strong>Add text</strong>
          <div style={{ flex: 1 }} />
          <button aria-label="Close" style={btn} onClick={onClose}>
            Close
          </button>
        </div>

        {fonts.kind === "loading" && <span style={{ fontSize: 12 }}>Listing fonts…</span>}
        {fonts.kind === "empty" && (
          <span style={{ fontSize: 12 }}>No fonts were found on this system</span>
        )}
        {fonts.kind === "error" && (
          <span style={{ fontSize: 12, color: "var(--cut)" }}>{fonts.message}</span>
        )}
        {fonts.kind === "ready" && (
          <label style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
            <span>Font</span>
            <select
              aria-label="Font family"
              style={{ flex: 1 }}
              value={fonts.selected}
              onChange={(e) => setFonts((f) => selectFamily(f, e.target.value))}
            >
              {fonts.families.map((family) => (
                <option key={family} value={family}>
                  {family}
                </option>
              ))}
            </select>
          </label>
        )}

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <div style={{ flex: 1 }} />
          <button aria-label="Cancel" style={btn} onClick={onClose}>
            Cancel
          </button>
          <button
            aria-label="Insert"
            style={btn}
            disabled={!canInsert(fonts)}
            onClick={() => fonts.kind === "ready" && onInsert(fonts.selected)}
          >
            Insert
          </button>
        </div>
      </div>
    </div>
  );
}
