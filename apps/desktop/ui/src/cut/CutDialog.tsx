// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useState, type CSSProperties } from "react";
import * as ipc from "../ipc";
import type { Scene } from "../render/hittest";
import { CutPreview } from "./CutPreview";
import {
  reorderPass,
  effectiveSettings,
  fieldDisabled,
  toCutRequest,
  type PassVm,
  type Caps,
  type Preset,
} from "./viewmodel";

// ponytail: no IPC exposes machine capabilities yet. Hardcoded from spec §5 — the Puma's
// speed/force are set on its own panel, so those fields stay disabled with a hint; the
// Cameo exposes both over the wire. Replace with a real capability query if more machines
// or knob variety show up.
const CAPS: Record<string, Caps> = {
  cameo5: { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false },
  puma: { supportsSpeed: false, supportsForce: false, needsOperatorPassConfirm: true },
};
const DEFAULT_CAPS: Caps = { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false };

type PassRow = PassVm & { nodeIds: number[] };

function stateLabel(s: ipc.DeviceState): string {
  if (typeof s === "string") return s;
  if ("Transmitting" in s) return "sending";
  if ("AwaitingCompletion" in s) return "awaiting completion";
  if ("WaitingForColorSwap" in s) return "Waiting for color swap";
  if ("CancelRequested" in s) return "cancel requested";
  if ("Stopping" in s) return "stopping";
  if ("Cancelled" in s) return "cancelled";
  if ("Error" in s) return "error";
  return "unknown";
}

type Props = {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  docMachineId: string | null;
  deviceState: ipc.DeviceState;
  lastEvent: ipc.DeviceEvent | null;
  jobId: number | null;
  setJobId: (id: number | null) => void;
  refreshDeviceState: () => void;
  onConvertMachine: (machineId: string) => void;
  onError: (msg: string) => void;
  onClose: () => void;
};

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

export function CutDialog({
  scene,
  artboard,
  docMachineId,
  deviceState,
  lastEvent,
  jobId,
  setJobId,
  refreshDeviceState,
  onConvertMachine,
  onError,
  onClose,
}: Props) {
  const [devices, setDevices] = useState<ipc.DeviceInfo[]>([]);
  const [connected, setConnected] = useState<ipc.DeviceInfo | null>(null);
  const [presets, setPresets] = useState<Preset[]>([]);
  const [rows, setRows] = useState<PassRow[]>([]);
  const [travel, setTravel] = useState<[number, number, number, number][]>([]);
  const [skippedNoStroke, setSkippedNoStroke] = useState(0);
  const [planRevision, setPlanRevision] = useState<string | null>(null);
  const [stalePlan, setStalePlan] = useState(false);

  useEffect(() => {
    ipc.listDevices().then(setDevices).catch((e) => onError(ipc.ipcErrorMessage(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const replan = () => {
    ipc
      .planCut()
      .then((plan) => {
        setRows(
          plan.passes.map((p) => ({
            color: p.color,
            shapeCount: p.shape_count,
            nodeIds: p.node_ids,
            enabled: true,
            presetId: null,
            speed: null,
            force: null,
            repeatCount: null,
          })),
        );
        setTravel(plan.travel);
        setSkippedNoStroke(plan.skipped_no_stroke);
        setPlanRevision(plan.doc_revision);
        setStalePlan(false);
      })
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  useEffect(() => {
    replan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const connect = (info: ipc.DeviceInfo) => {
    ipc
      .connectDevice(info)
      .then(() => {
        setConnected(info);
        refreshDeviceState();
        return ipc.listPresets(info.machine_id);
      })
      .then((p) => setPresets(p as Preset[]))
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  const caps = connected ? CAPS[connected.machine_id] ?? DEFAULT_CAPS : DEFAULT_CAPS;
  const machineMismatch = docMachineId !== null && connected !== null && docMachineId !== connected.machine_id;
  const idle = deviceState === "Idle";
  const waitingSwap = typeof deviceState === "object" && "WaitingForColorSwap" in deviceState;
  const awaitingCompletion = typeof deviceState === "object" && "AwaitingCompletion" in deviceState;
  const transmitting = typeof deviceState === "object" && "Transmitting" in deviceState;
  const active = transmitting || (typeof deviceState === "object" && ("CancelRequested" in deviceState || "Stopping" in deviceState));
  // Idle only reads as "job complete" once a job has actually run — the pre-cut Idle
  // state (jobId still null) must not show a stale completion message.
  const justCompleted = jobId !== null && idle;
  const failed = lastEvent && jobId !== null && lastEvent.job_id === jobId && typeof lastEvent.kind === "object" && "Failed" in lastEvent.kind;

  const startCut = () => {
    if (!connected || planRevision === null) return;
    const request = toCutRequest(connected.instance_id, planRevision, rows);
    ipc
      .cut(request)
      .then((id) => setJobId(id))
      .catch((e) => {
        const code = ipc.ipcErrorCode(e);
        if (code === "stale_plan") setStalePlan(true);
        onError(ipc.ipcErrorMessage(e));
      });
  };

  const resume = () => {
    ipc
      .resumeCut()
      .then(() => refreshDeviceState())
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  const cancel = () => {
    ipc
      .cancelCut()
      .then(() => refreshDeviceState())
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  const confirmPassDone = () => {
    ipc
      .confirmPassDone()
      .then(() => refreshDeviceState())
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  const updateRow = (i: number, patch: Partial<PassRow>) => {
    setRows((prev) => prev.map((r, idx) => (idx === i ? { ...r, ...patch } : r)));
  };

  return (
    <div style={panelStyle}>
      <div role="dialog" aria-modal="true" aria-label="Cut" style={dialogStyle}>
        <div style={{ display: "flex", alignItems: "center" }}>
          <strong>Cut</strong>
          <div style={{ flex: 1 }} />
          <button aria-label="Close" style={btn} onClick={onClose}>
            Close
          </button>
        </div>

        <div>
          <div style={{ fontSize: 12, color: "var(--muted)", marginBottom: 4 }}>Device</div>
          {devices.map((d) => (
            <div key={d.instance_id} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
              <span>
                {d.machine_id}
                {d.candidate ? " (unverified serial device)" : ""}
              </span>
              {connected?.instance_id === d.instance_id ? (
                <span style={{ color: "var(--ready)" }}>connected</span>
              ) : (
                <button style={btn} onClick={() => connect(d)}>
                  Connect
                </button>
              )}
            </div>
          ))}
        </div>

        {machineMismatch && connected ? (
          <div style={{ color: "var(--cut)", fontSize: 12, display: "flex", alignItems: "center", gap: 8 }}>
            Document is set up for a different machine than the connected device.
            <button style={btn} onClick={() => onConvertMachine(connected.machine_id)}>
              Convert to {connected.machine_id}
            </button>
          </div>
        ) : null}

        {stalePlan ? (
          <div style={{ color: "var(--cut)", fontSize: 12, display: "flex", alignItems: "center", gap: 8 }}>
            Document changed since this plan was made.
            <button style={btn} onClick={replan}>
              Replan
            </button>
          </div>
        ) : null}

        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {rows.map((row, i) => {
            const eff = effectiveSettings(row, presets);
            const speedDisabled = fieldDisabled("speed", caps);
            const forceDisabled = fieldDisabled("force", caps);
            return (
              <div
                key={i}
                data-testid="cut-pass-row"
                style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12, border: "1px solid var(--border)", padding: 6 }}
              >
                <span
                  style={{
                    width: 12,
                    height: 12,
                    display: "inline-block",
                    background: row.color !== null ? `#${(row.color >>> 8).toString(16).padStart(6, "0")}` : "var(--muted)",
                  }}
                />
                <span>{row.shapeCount} shape(s)</span>
                <label>
                  <input type="checkbox" checked={row.enabled} onChange={(e) => updateRow(i, { enabled: e.target.checked })} />
                  Enabled
                </label>
                <select
                  aria-label={`Preset for pass ${i + 1}`}
                  value={row.presetId ?? ""}
                  onChange={(e) => updateRow(i, { presetId: e.target.value || null })}
                >
                  <option value="">No preset</option>
                  {presets.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name}
                    </option>
                  ))}
                </select>
                <input
                  aria-label={`Speed for pass ${i + 1}`}
                  type="number"
                  disabled={speedDisabled}
                  value={eff.speed ?? ""}
                  placeholder="speed"
                  onChange={(e) => updateRow(i, { speed: e.target.value === "" ? null : Number(e.target.value) })}
                  style={{ width: 60 }}
                />
                <input
                  aria-label={`Force for pass ${i + 1}`}
                  type="number"
                  disabled={forceDisabled}
                  value={eff.force ?? ""}
                  placeholder="force"
                  onChange={(e) => updateRow(i, { force: e.target.value === "" ? null : Number(e.target.value) })}
                  style={{ width: 60 }}
                />
                {speedDisabled || forceDisabled ? <span style={{ color: "var(--muted)" }}>set on the Puma's panel</span> : null}
                <button style={btn} onClick={() => setRows(reorderPass(rows, i, -1) as PassRow[])} disabled={i === 0}>
                  Up
                </button>
                <button style={btn} onClick={() => setRows(reorderPass(rows, i, 1) as PassRow[])} disabled={i === rows.length - 1}>
                  Down
                </button>
              </div>
            );
          })}
        </div>

        <div style={{ fontSize: 12, color: "var(--muted)" }}>Not cut: {skippedNoStroke} shapes</div>

        <CutPreview scene={scene} artboard={artboard} passes={rows} travel={travel} />

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {waitingSwap ? <span>Waiting for color swap</span> : null}
          {transmitting && typeof deviceState === "object" && "Transmitting" in deviceState ? (
            <span>
              sending {deviceState.Transmitting.submitted_bytes} / {deviceState.Transmitting.total_bytes} bytes
            </span>
          ) : active ? (
            <span>{stateLabel(deviceState)}</span>
          ) : null}
          {awaitingCompletion ? <span>Awaiting completion</span> : null}
          {justCompleted ? <span>Job complete</span> : null}
          {failed ? <span style={{ color: "var(--cut)" }}>Cut failed</span> : null}

          <div style={{ flex: 1 }} />

          {waitingSwap ? (
            <button aria-label="Resume" style={btn} onClick={resume}>
              Resume
            </button>
          ) : null}
          {awaitingCompletion ? (
            <button aria-label="Confirm pass done" style={btn} onClick={confirmPassDone}>
              Confirm pass done
            </button>
          ) : null}
          {active ? (
            <button aria-label="Cancel" style={btn} onClick={cancel}>
              Cancel
            </button>
          ) : null}
          {!waitingSwap && !awaitingCompletion && !active ? (
            <button
              aria-label="Start Cut"
              style={btn}
              disabled={!connected || !idle || machineMismatch || rows.length === 0}
              onClick={startCut}
            >
              Start Cut
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
