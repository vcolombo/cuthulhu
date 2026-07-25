// SPDX-License-Identifier: GPL-3.0-or-later
import { useRef, type CSSProperties } from "react";
import type { MachineProfile } from "../App";

type Props = {
  machines: MachineProfile[];
  currentMachineId: string | null;
  onSelectMachine: (id: string) => void;
  onSave: () => void;
  onOpen: () => void;
  onReload: () => void;
  canReload: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onImportFile: (file: File) => void;
  onCut: () => void;
};

const btn: CSSProperties = {
  background: "var(--panel)",
  color: "var(--text)",
  border: "1px solid var(--border)",
  padding: "4px 10px",
  cursor: "pointer",
};

export function TopBar({
  machines,
  currentMachineId,
  onSelectMachine,
  onSave,
  onOpen,
  onReload,
  canReload,
  onUndo,
  onRedo,
  onImportFile,
  onCut,
}: Props) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 10px",
        background: "var(--panel)",
        borderBottom: "1px solid var(--border)",
      }}
    >
      <span style={{ color: "var(--text)", fontWeight: 600, fontSize: 13, marginRight: 8 }}>cuthulhu</span>
      <button aria-label="Undo" style={btn} onClick={onUndo}>
        Undo
      </button>
      <button aria-label="Redo" style={btn} onClick={onRedo}>
        Redo
      </button>
      <button aria-label="Import" style={btn} onClick={() => fileInputRef.current?.click()}>
        Import
      </button>
      <input
        ref={fileInputRef}
        type="file"
        accept=".svg"
        style={{ display: "none" }}
        onChange={(e) => {
          const file = e.target.files?.[0];
          e.target.value = ""; // allow re-selecting the same file
          if (file) onImportFile(file);
        }}
      />
      <div style={{ flex: 1 }} />
      <select
        aria-label="Machine"
        value={currentMachineId ?? ""}
        onChange={(e) => onSelectMachine(e.target.value)}
        style={{ background: "var(--workspace)", color: "var(--text)", border: "1px solid var(--border)" }}
      >
        <option value="" disabled>
          Select machine
        </option>
        {machines.map((m) => (
          <option key={m.id} value={m.id}>
            {m.name}
          </option>
        ))}
      </select>
      <button aria-label="Save" style={btn} onClick={onSave}>
        Save
      </button>
      <button aria-label="Open" style={btn} onClick={onOpen}>
        Open
      </button>
      <button aria-label="Reload" style={btn} onClick={onReload} disabled={!canReload}>
        Reload
      </button>
      <button aria-label="Cut" style={btn} onClick={onCut}>
        Cut
      </button>
    </div>
  );
}
