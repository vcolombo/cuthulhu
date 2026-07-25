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
  dialogPhase,
  canStartCut,
  dialogButtons,
  type PassVm,
  type Caps,
  type Preset,
} from "./viewmodel";

// ponytail: no IPC exposes machine capabilities yet (follow-up: expose MachineCaps via
// IPC). Hardcoded from spec §5 — the Puma's speed/force are set on its own panel, so
// those fields stay disabled with a hint; the Cameo exposes both over the wire. Replace
// with a real capability query if more machines or knob variety show up.
const CAPS: Record<string, Caps> = {
  cameo5: { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false },
  puma: { supportsSpeed: false, supportsForce: false, needsOperatorPassConfirm: true },
};
const DEFAULT_CAPS: Caps = { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false };

type PassRow = PassVm & { nodeIds: number[] };

type Props = {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  docMachineId: string | null;
  deviceState: ipc.DeviceState;
  cutOutcome: "complete" | "failed" | null;
  clearCutOutcome: () => void;
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
  cutOutcome,
  clearCutOutcome,
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
    // Reopening the dialog after a connect earlier in the session lost the local
    // `connected` state (it lives only in this component) even though the backend
    // is still connected — seed it from the manager's own cache so Start Cut isn't
    // stuck disabled. Presets load with it: they're otherwise only fetched in
    // connect(), so a reopened dialog would show an empty preset dropdown.
    ipc
      .getConnectedDevice()
      .then((info) => {
        setConnected(info);
        if (!info) return;
        return ipc.listPresets(info.machine_id).then((p) => setPresets(p as Preset[]));
      })
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
    // A stale jobId or latched outcome from a previous dialog session must not
    // leak into this one: reopening the dialog starts fresh, so a prior cut's
    // "Job complete"/"Cut failed" banner doesn't reappear.
    setJobId(null);
    clearCutOutcome();
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
  const phase = dialogPhase(deviceState);
  const buttons = dialogButtons(phase);
  // The banner keys off cutOutcome, which App.tsx latches from the job's own
  // terminal event — decoupled from jobId, which is the *event-filter* id and is
  // released the moment the job ends (so NO_JOB=0 lifecycle events keep flowing).
  // Deriving the banner from jobId + an Idle state was wrong twice over: it either
  // flashed for one render (jobId cleared on completion) or showed "Job complete"
  // for a *failed* job whose lagging state cache read Idle (jobId retained).
  // Cleared on mount (above) and on every new cut (startCut below).
  const justCompleted = cutOutcome === "complete";
  const failed = cutOutcome === "failed";

  const startCut = () => {
    if (!connected || planRevision === null) return;
    // Resets on every new cut() call: without this, a second cut in the same
    // dialog session keeps the finished first job's id around, so acceptEvent
    // (App.tsx) rejects every event the new job emits until it happens to reuse
    // the old id (it never does — ids are strictly increasing). The previous
    // outcome banner also clears — a new cut supersedes it.
    setJobId(null);
    clearCutOutcome();
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
                key={row.color ?? "none"}
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
                <input
                  aria-label={`Repeat count for pass ${i + 1}`}
                  type="number"
                  min={1}
                  value={eff.repeatCount}
                  placeholder="repeat"
                  onChange={(e) => updateRow(i, { repeatCount: e.target.value === "" ? null : Number(e.target.value) })}
                  style={{ width: 50 }}
                />
                {speedDisabled || forceDisabled ? <span style={{ color: "var(--muted)" }}>set on the Puma's panel</span> : null}
                <button style={btn} onClick={() => setRows(reorderPass(rows, i, -1))} disabled={i === 0}>
                  Up
                </button>
                <button style={btn} onClick={() => setRows(reorderPass(rows, i, 1))} disabled={i === rows.length - 1}>
                  Down
                </button>
              </div>
            );
          })}
        </div>

        <div style={{ fontSize: 12, color: "var(--muted)" }}>Not cut: {skippedNoStroke} shapes</div>

        <CutPreview scene={scene} artboard={artboard} passes={rows} travel={travel} />

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {phase.kind === "waitingSwap" ? <span>Waiting for color swap</span> : null}
          {phase.kind === "transmitting" ? (
            <span>
              sending {phase.submittedBytes} / {phase.totalBytes} bytes
            </span>
          ) : null}
          {phase.kind === "cancelRequested" ? <span>cancel requested</span> : null}
          {phase.kind === "stopping" ? <span>stopping</span> : null}
          {phase.kind === "awaitingCompletion" ? <span>Awaiting completion</span> : null}
          {phase.kind === "cancelled" ? <span style={{ color: "var(--muted)" }}>Cancelled</span> : null}
          {justCompleted ? <span>Job complete</span> : null}
          {failed ? <span style={{ color: "var(--cut)" }}>Cut failed</span> : null}

          <div style={{ flex: 1 }} />

          {buttons.resume ? (
            <button aria-label="Resume" style={btn} onClick={resume}>
              Resume
            </button>
          ) : null}
          {buttons.confirmPassDone ? (
            <button aria-label="Confirm pass done" style={btn} onClick={confirmPassDone}>
              Confirm pass done
            </button>
          ) : null}
          {buttons.cancel ? (
            <button aria-label="Cancel" style={btn} onClick={cancel}>
              Cancel
            </button>
          ) : null}
          {buttons.start ? (
            <button
              aria-label="Start Cut"
              style={btn}
              disabled={!connected || !canStartCut(phase) || machineMismatch || rows.length === 0}
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
