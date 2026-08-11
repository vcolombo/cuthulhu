// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useState, type CSSProperties } from "react";
import * as ipc from "../ipc";
import { deviceBadge, forgetFrom, groupDevices } from "../hosts/deviceList";
import { PairHostDialog } from "../hosts/PairHostDialog";
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

// What the fields allow before any machine has been asked. Not a machine's claim —
// a placeholder that keeps passes editable offline. Preflight ignores speed/force a
// machine does not support, so an optimistic default here cannot mis-send anything.
const ALL_ENABLED: Caps = { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false };

type PassRow = PassVm & { nodeIds: number[] };

type Props = {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  docMachineId: string | null;
  status: ipc.CutStatus;
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
  status,
  refreshDeviceState,
  onConvertMachine,
  onError,
  onClose,
}: Props) {
  const [devices, setDevices] = useState<ipc.DeviceInfo[]>([]);
  const [hosts, setHosts] = useState<ipc.PairedHostView[]>([]);
  const [pairing, setPairing] = useState(false);
  const [connected, setConnected] = useState<ipc.DeviceInfo | null>(null);
  // The machine id rides along with the caps: `connected` can change before an
  // in-flight fetch resolves, and showing one machine's capability against another
  // is the exact defect this fetch was added to remove.
  const [capsFor, setCapsFor] = useState<{ machineId: string; caps: Caps } | null>(null);
  const [presets, setPresets] = useState<Preset[]>([]);
  const [rows, setRows] = useState<PassRow[]>([]);
  const [travel, setTravel] = useState<[number, number, number, number][]>([]);
  const [skippedNoStroke, setSkippedNoStroke] = useState(0);
  const [planRevision, setPlanRevision] = useState<string | null>(null);
  const [stalePlan, setStalePlan] = useState(false);

  useEffect(() => {
    ipc.listDevices().then(setDevices).catch((e) => onError(ipc.ipcErrorMessage(e)));
    // Separate chain from the devices above, for the same reason the caps and presets fetches
    // are separate: a Cut Host that cannot be listed must not blank the local cutters, which are
    // the ones still usable when it is.
    ipc.listHosts().then(setHosts).catch((e) => onError(ipc.ipcErrorMessage(e)));
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
        // Separate chains on purpose: a corrupt presets file must not leave caps
        // unfetched, and an unknown machine must not blank the preset dropdown.
        ipc
          .machineCaps(info.machine_id)
          .then((c) => setCapsFor({ machineId: info.machine_id, caps: c as Caps }))
          .catch((e) => onError(ipc.ipcErrorMessage(e)));
        return ipc.listPresets(info.machine_id).then((p) => setPresets(p as Preset[]));
      })
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
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
        ipc
          .machineCaps(info.machine_id)
          .then((c) => setCapsFor({ machineId: info.machine_id, caps: c as Caps }))
          .catch((e) => onError(ipc.ipcErrorMessage(e)));
        return ipc.listPresets(info.machine_id);
      })
      .then((p) => setPresets(p as Preset[]))
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  // A refusal keeps the row and shows the host's own words: the Rust side refuses with
  // `host_busy` while a cut is active there, and a row that vanished and came back would say
  // "gone" about the one host that still has a blade moving.
  const forget = (id: string) => {
    forgetFrom(hosts, id, ipc.forgetHost).then((r) => {
      setHosts(r.hosts);
      if (r.message !== null) onError(r.message);
    });
  };

  const paired = (host: ipc.PairedHostView) => {
    setHosts((prev) => [...prev, host]);
    setPairing(false);
    // The host's cutters are only in the device list once it has been re-read; `runPairing`'s
    // Test listed them to prove the token, not to populate this.
    ipc.listDevices().then(setDevices).catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  const caps = connected && capsFor?.machineId === connected.machine_id ? capsFor.caps : ALL_ENABLED;
  const machineMismatch = docMachineId !== null && connected !== null && docMachineId !== connected.machine_id;

  const startCut = () => {
    if (!connected || planRevision === null) return;
    const request = toCutRequest(connected.instance_id, planRevision, rows);
    ipc.cut(request).catch((e) => {
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
    <>
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
          {/* Only when there is nothing at all. A paired host with no cutters attached is a
              different fact, and it has its own section saying so. */}
          {devices.length === 0 && hosts.length === 0 ? (
            <div style={{ fontSize: 12, color: "var(--muted)" }}>
              No devices found — connect a cutter and reopen this dialog.
            </div>
          ) : null}
          {groupDevices(devices, hosts).map((section) => (
            <div key={section.hostId ?? "local"} style={{ marginBottom: 6 }}>
              {/* The local section's header is suppressed when it is the only one, so a user with
                  no Cut Host sees the flat list this dialog has always shown. */}
              {section.hostId === null && hosts.length === 0 ? null : (
                <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
                  <strong>{section.title}</strong>
                  {section.address ? <span style={{ color: "var(--muted)" }}>{section.address}</span> : null}
                  {section.stale ? <span style={{ color: "var(--muted)" }}>last known</span> : null}
                  <div style={{ flex: 1 }} />
                  {hosts.some((h) => h.id === section.hostId) ? (
                    <button
                      aria-label={`Forget ${section.title}`}
                      style={btn}
                      onClick={() => forget(section.hostId!)}
                    >
                      Forget
                    </button>
                  ) : null}
                </div>
              )}
              {/* An unreachable host keeps its cutters listed below this, not hidden (#42). */}
              {section.unreachable ? (
                <div style={{ fontSize: 12, color: "var(--cut)" }}>{section.unreachable}</div>
              ) : null}
              {section.devices.map((d) => (
                <div key={d.instance_id} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
                  <span>
                    {d.machine_id}
                    {d.candidate ? " (unverified serial device)" : ""}
                  </span>
                  {/* Only the aimed-at cutter has a status; the rest have not been asked, and
                      `null` is what says so rather than something that reads as ready. */}
                  <span style={{ color: "var(--muted)" }}>
                    {deviceBadge(connected?.instance_id === d.instance_id ? status : null).label}
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
          ))}
          {/* Pairing lives in the device list on purpose: someone hunting for their Pi looks
              here, and finds nothing if it lives in a settings screen. */}
          <button style={btn} onClick={() => setPairing(true)}>
            Add a Cut Host…
          </button>
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
          {status.phase === "AwaitingColorSwap" ? <span>Waiting for color swap</span> : null}
          {status.phase === "Sending" && status.sent ? (
            <span>
              sending {status.sent.sent} / {status.sent.total} bytes
            </span>
          ) : null}
          {status.phase === "Cancelling" ? <span>cancelling</span> : null}
          {status.phase === "AwaitingConfirmation" ? <span>Awaiting completion</span> : null}
          {/* How the last job ended comes from the backend, so the dialog remembers
              nothing: no latch, no reading `actions.cancel` as a liveness bit. */}
          {status.ended === "Cancelled" ? <span style={{ color: "var(--muted)" }}>Cancelled</span> : null}
          {status.ended === "Completed" ? <span>Job complete</span> : null}
          {status.phase === "Failed" ? <span style={{ color: "var(--cut)" }}>Cut failed</span> : null}

          <div style={{ flex: 1 }} />

          {status.actions.resume ? (
            <button aria-label="Resume" style={btn} onClick={resume}>
              Resume
            </button>
          ) : null}
          {status.actions.confirm ? (
            <button aria-label="Confirm pass done" style={btn} onClick={confirmPassDone}>
              Confirm pass done
            </button>
          ) : null}
          {status.actions.cancel ? (
            <button aria-label="Cancel" style={btn} onClick={cancel}>
              Cancel
            </button>
          ) : null}
          {/* The only enablement checks left are the ones the backend cannot know:
              whether this dialog has a device, a matching machine and rows to cut. */}
          <button
            aria-label="Start Cut"
            style={btn}
            disabled={!status.actions.cut || !connected || machineMismatch || rows.length === 0}
            onClick={startCut}
          >
            Start Cut
          </button>
        </div>
      </div>
    </div>
    {pairing ? <PairHostDialog onPaired={paired} onClose={() => setPairing(false)} /> : null}
    </>
  );
}
