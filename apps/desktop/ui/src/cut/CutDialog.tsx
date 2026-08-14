// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import * as ipc from "../ipc";
import { connectedControl, deviceBadge, forgetFrom, groupDevices, sameCutter, staleSection } from "../hosts/deviceList";
import { PairHostDialog } from "../hosts/PairHostDialog";
import type { Scene } from "../render/hittest";
import { CutPreview } from "./CutPreview";
import {
  reorderForReplan,
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

type PassRow = PassVm & { nodeIds: number[]; starts: ([number, number] | null)[] };

type Props = {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  docMachineId: string | null;
  status: ipc.CutStatus;
  refreshDeviceState: () => Promise<void>;
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

// `deviceBadge` decides the tone; the palette is this file's, since a pure module has no
// business knowing the tokens. Without this the label rendered in one flat grey, so "Cutting",
// "Ready" and "Unreachable" were the same small muted text as the machine id beside them —
// six cutters across two hosts with one mid-cut, and nothing on screen picking it out.
// Red belongs to `attention` alone, which is the tone that asks for a person. `unknown` is the
// state of every cutter on a freshly opened dialog, so it keeps the muted weight the row had
// before any of this: quiet is what "nobody has asked yet" should look like.
const TONE_COLOR: Record<ReturnType<typeof deviceBadge>["tone"], string> = {
  idle: "var(--ready)",
  busy: "var(--accent)",
  attention: "var(--cut)",
  unknown: "var(--muted)",
  gone: "var(--muted)",
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
  // What the last read of the two lists failed with, held rather than reported: it is what marks
  // the rows stale instead of blanking them.
  const [listError, setListError] = useState<unknown>(null);
  const [pairing, setPairing] = useState(false);
  /** The host whose forget was refused because nothing could confirm it is idle, if any. */
  const [forceHost, setForceHost] = useState<string | null>(null);
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
  /** Set when the Cut Host answered that it had already accepted this dispatch, so nothing new
   *  started. Cleared by the next press of Cut, which is the thing that makes it untrue. */
  const [alreadyAccepted, setAlreadyAccepted] = useState(false);
  /** A cut request is out. The backend reserves one dispatch id per Job, so a second click is
   *  safe — it joins the first rather than starting a second Job — but it would come back
   *  "already accepted", which is a confusing thing to say about a double-click. */
  const [cutInFlight, setCutInFlight] = useState(false);

  // The whole device list in one request rather than one per host: `list_devices` already
  // re-reads every paired host in a single call, and `list_hosts` carries why any of them cannot
  // be reached.
  const refreshList = useCallback(
    () =>
      Promise.all([ipc.listDevices(), ipc.listHosts()])
        .then(([d, h]) => {
          setDevices(d);
          setHosts(h);
          setListError(null);
        })
        // Kept, not raised: a read that failed leaves the last values it managed on screen,
        // marked stale. A row that blanks reads as "the cut finished", which is the one wrong
        // thing to tell someone whose material is still moving under a blade.
        .catch((e) => setListError(e)),
    [],
  );

  useEffect(() => {
    refreshList();
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

  /** Serial number of the newest travel-affecting request; see movePass and replan. */
  const travelSeq = useRef(0);

  const replan = () => {
    // A fresh plan orphans every in-flight reorder request: bumping the sequence here
    // keeps a slow reply planned against the old revision from overwriting the new
    // travel — or re-raising the stale banner the fresh plan just cleared.
    travelSeq.current++;
    ipc
      .planCut()
      .then((plan) => {
        setRows(
          plan.passes.map((p) => ({
            color: p.color,
            shapeCount: p.shape_count,
            nodeIds: p.node_ids,
            starts: p.starts,
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

  // A cut on a Cut Host is watched by asking, not by being told: nothing pushes over the
  // request/reply connection, so this interval is the only thing that moves a remote cutter's
  // progress. It runs exactly as long as this dialog is mounted and a host is paired.
  //
  // With no host there is no gap to fill — a local cutter's status is pushed by the
  // device-event listener in `App.tsx` — and the cost is not nothing: `list_devices` resolves
  // to `HardwareBackendFactory::list_devices`, which walks the USB bus and the serial ports.
  // The Pi is optional, and a desktop without one pays for it once per dialog, as before.
  //
  // Clearing it is the deliverable, not tidiness. A leaked interval keeps a Cut Host connection
  // warm forever and the daemon caps concurrent clients at eight (#103), so one leaked per
  // dialog-open exhausts a Pi, which then refuses every new connection until it is restarted.
  useEffect(() => {
    if (hosts.length === 0) return;
    // ponytail: one interval for the whole list, not one per host — the two calls behind it each
    // cover every host at once. Per-host intervals are the upgrade if one slow host ever starts
    // holding up the others' rows, and they cost a teardown each.
    let inFlight = false;
    const id = setInterval(() => {
      // Skipped rather than queued: an unreachable host can take seconds to answer, and ticks
      // that wait their turn build a backlog that outlives whatever wedged the host.
      if (inFlight) return;
      inFlight = true;
      Promise.all([refreshList(), refreshDeviceState()]).finally(() => {
        inFlight = false;
      });
    }, 1000);
    return () => clearInterval(id);
    // Length, not the array: a poll that replaces `hosts` with an equal list every second would
    // otherwise tear the interval down and rebuild it every second.
  }, [refreshList, refreshDeviceState, hosts.length]);

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

  // Clears `connected` locally too: `disconnect_device` is what releases the local manager, and a
  // row still labelled "connected" would offer no way back to the Connect that clears the state.
  const disconnect = () => {
    ipc
      .disconnectDevice()
      .then(() => {
        setConnected(null);
        refreshDeviceState();
      })
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  // Keeps the aim on purpose, unlike `disconnect`: the cutter is still there and still this
  // desktop's, it has simply had its transport re-opened. The status refresh is the point —
  // `actions.cut` is what was withheld and what should come back.
  const reconnect = () => {
    ipc
      .reconnectDevice()
      .then(refreshDeviceState)
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  // A refusal keeps the row and shows the host's own words: the Rust side refuses while a cut is
  // active there and whenever it cannot ask, and a row that vanished and came back would say
  // "gone" about the one host that might still have a blade moving.
  // A success re-reads both lists rather than dropping the host here. `devices` still holds that
  // host's cutters, and a cutter naming a host nobody is paired with is precisely what earns an
  // orphan section — so removing the row on its own renamed it to a raw host id and captioned it
  // "not paired with this computer", which is a warning about a host the operator just dismissed.
  //
  // The force is offered by `forceHost`, and only for the host whose unforced attempt just came
  // back `host_unconfirmed` — never standing there ahead of the try that would explain why it
  // exists.
  const forget = (id: string, force = false) => {
    forgetFrom(hosts, id, ipc.forgetHost, force).then((r) => {
      setForceHost(r.forceable ? id : null);
      if (r.message === null) refreshList();
      else onError(r.message);
    });
  };

  // Both lists come back from the backend, which has just persisted the host — the new row is
  // never assembled on this side. Appending it here put the same Pi in the list twice, listed
  // under one name and forgettable once. Its cutters arrive the same way: `runPairing`'s Test
  // listed them to prove the token, not to populate this.
  const paired = () => {
    setPairing(false);
    refreshList();
  };

  // A read that failed keeps every section it had, marked stale, rather than emptying the list.
  const sections = groupDevices(devices, hosts);
  const shown = listError === null ? sections : sections.map((s) => staleSection(s, listError));

  const caps = connected && capsFor?.machineId === connected.machine_id ? capsFor.caps : ALL_ENABLED;
  const machineMismatch = docMachineId !== null && connected !== null && docMachineId !== connected.machine_id;

  const startCut = () => {
    if (!connected || planRevision === null) return;
    const request = toCutRequest(connected.instance_id, planRevision, rows);
    setAlreadyAccepted(false);
    setCutInFlight(true);
    ipc
      .cut(request)
      // Not an error, and not a success worth saying nothing about: the host recognised this
      // dispatch and started nothing. Told apart, because a cutter that is not moving looks the
      // same either way and only the host knows which happened.
      .then((started) => setAlreadyAccepted(started.duplicate))
      .catch((e) => {
        const code = ipc.ipcErrorCode(e);
        if (code === "stale_plan") setStalePlan(true);
        onError(ipc.ipcErrorMessage(e));
      })
      .finally(() => setCutInFlight(false));
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

  // Rapid Up/Up fires two replans, and nothing orders their replies — an older
  // response landing last would redraw travel for an order the rows no longer show.
  // Only the latest request's reply (or failure) is allowed to touch state, and
  // `replan` bumps the sequence too, so a full replan orphans them all.
  const movePass = (i: number, dir: -1 | 1) => {
    const moved = reorderForReplan(rows, i, dir);
    if (!moved) return;
    setRows(moved.rows);
    // No plan means no travel on screen to go stale (the initial plan itself failed).
    if (planRevision === null) return;
    const seq = ++travelSeq.current;
    ipc
      .travelForOrder(planRevision, moved.order)
      .then((t) => {
        if (seq === travelSeq.current) setTravel(t);
      })
      .catch((e) => {
        if (seq !== travelSeq.current) return;
        // The same refusal Start Cut gets, surfaced the same way: the banner's Replan
        // is the way back, and the reordered rows keep the operator's arrangement.
        if (ipc.ipcErrorCode(e) === "stale_plan") setStalePlan(true);
        else onError(ipc.ipcErrorMessage(e));
      });
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
          {shown.map((section) => (
            <div key={section.hostId ?? "local"} style={{ marginBottom: 6 }}>
              {/* The local section's header is suppressed when it is the only one, so a user with
                  no Cut Host sees the flat list this dialog has always shown — but not when the
                  read failed. The "last known" marker lives in this header, so suppressing it
                  left the reason below as an unlabelled red line of Rust prose under "Device",
                  with nothing saying which list it was about or that the rows had stopped
                  moving. A failure the user cannot place is worse than a heading they did not
                  need. */}
              {section.hostId === null && hosts.length === 0 && !section.stale ? null : (
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
              {/* Says what is being accepted rather than asking "are you sure": the risk is not
                  that the host is gone, it is that it is still cutting and this is the only
                  desktop that could stop it. Shown only after the plain Forget was refused. */}
              {forceHost === section.hostId ? (
                <div style={{ fontSize: 12, color: "var(--cut)", display: "flex", alignItems: "center", gap: 8 }}>
                  <span>
                    A cut may still be running on this Cut Host. Forgetting it discards the
                    credentials this desktop needs to cancel, resume or confirm that cut — it will
                    not be able to stop it.
                  </span>
                  {/* Not "Forget <name> anyway": that name contains this section's own Forget
                      button's, and a selector for one would match both. */}
                  <button
                    aria-label={`Discard ${section.title} anyway`}
                    style={btn}
                    onClick={() => forget(section.hostId!, true)}
                  >
                    Discard anyway
                  </button>
                  <button aria-label={`Keep ${section.title}`} style={btn} onClick={() => setForceHost(null)}>
                    Keep it
                  </button>
                </div>
              ) : null}
              {section.devices.map((d) => {
                // Only the aimed-at cutter has a status; the rest have not been asked, and
                // `null` is what says so rather than something that reads as ready. Matched on the
                // id *and* the host: two hosts wired alike hand out the same fallback id, and
                // giving one host's status to the other's row hands Cancel to the wrong machine.
                const isAimed = sameCutter(connected, d);
                const aimed = isAimed ? status : null;
                const badge = deviceBadge(aimed);
                const control = connectedControl(aimed, d.host !== null);
                return (
                <div key={`${d.host ?? "local"}:${d.instance_id}`} style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 12 }}>
                  <span>
                    {d.machine_id}
                    {d.candidate ? " (unverified serial device)" : ""}
                  </span>
                  <span data-testid="device-badge" data-tone={badge.tone} style={{ color: TONE_COLOR[badge.tone] }}>
                    {badge.label}
                  </span>
                  {isAimed ? (
                    <>
                      <span style={{ color: "var(--ready)" }}>connected</span>
                      {/* The only way back from a stop nothing confirmed: `driver-core` refuses
                          both a cut and a connect from that state, so without this the operator's
                          exit is restarting the app. Withheld mid-Job — see `connectedControl`. */}
                      {control ? (
                        <button
                          aria-label={`${control.label} ${d.instance_id}`}
                          style={btn}
                          onClick={() => (control.verb === "reconnect" ? reconnect() : disconnect())}
                        >
                          {control.label}
                        </button>
                      ) : null}
                    </>
                  ) : (
                    <button style={btn} onClick={() => connect(d)}>
                      Connect
                    </button>
                  )}
                </div>
                );
              })}
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

        {alreadyAccepted ? (
          <div role="alert" style={{ color: "var(--cut)", fontSize: 12 }}>
            This Cut Host had already accepted this cut, so nothing new was started — this was read
            as a retry of a dispatch whose answer was lost. If the cutter is idle and you meant a
            fresh sheet, press Cut again.
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
                <button style={btn} onClick={() => movePass(i, -1)} disabled={i === 0}>
                  Up
                </button>
                <button style={btn} onClick={() => movePass(i, 1)} disabled={i === rows.length - 1}>
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
            disabled={!status.actions.cut || !connected || machineMismatch || rows.length === 0 || cutInFlight}
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
