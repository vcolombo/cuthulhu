// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useState } from "react";
import * as ipc from "../../ipc";
import { connectedControl, deviceBadge, forgetFrom, groupDevices, sameCutter, staleSection } from "../../hosts/deviceList";
import { PairHostDialog } from "../../hosts/PairHostDialog";
import { btn, TONE_COLOR } from "./chrome";

/** The device list and the poll that keeps a Cut Host's rows moving. Called from the sheet so its
 *  mount effect stays ahead of the plan: the list and the ranges used to start in one effect. */
export function useDeviceList({
  refreshDeviceState,
  onError,
  onReconnect,
}: {
  refreshDeviceState: () => Promise<void>;
  onError: (msg: string) => void;
  onReconnect: () => Promise<unknown>;
}) {
  const [devices, setDevices] = useState<ipc.DeviceInfo[]>([]);
  const [hosts, setHosts] = useState<ipc.PairedHostView[]>([]);
  // What the last read of the two lists failed with, held rather than reported: it is what marks
  // the rows stale instead of blanking them.
  const [listError, setListError] = useState<unknown>(null);
  const [pairing, setPairing] = useState(false);
  /** The host whose forget was refused because nothing could confirm it is idle, if any. */
  const [forceHost, setForceHost] = useState<string | null>(null);

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
    void refreshList();
  }, [refreshList]);

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

  // Keeps the aim on purpose, unlike `disconnect`: the cutter is still there and still this
  // desktop's, it has simply had its transport re-opened. The status refresh is the point —
  // `actions.cut` is what was withheld and what should come back.
  const reconnect = () => {
    onReconnect()
      .then(() => undefined)
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  // A read that failed keeps every section it had, marked stale, rather than emptying the list.
  const sections = groupDevices(devices, hosts);
  const shown = listError === null ? sections : sections.map((s) => staleSection(s, listError));

  return { devices, hosts, shown, forceHost, setForceHost, pairing, setPairing, forget, paired, reconnect };
}

export function HostPairing({
  open,
  onPaired,
  onClose,
}: {
  open: boolean;
  onPaired: () => void;
  onClose: () => void;
}) {
  if (!open) return null;
  return <PairHostDialog onPaired={onPaired} onClose={onClose} />;
}

export function StepMachine({
  devices,
  hosts,
  shown,
  forceHost,
  setForceHost,
  connected,
  status,
  aiming,
  guardUnsaved,
  onConnect,
  onDisconnect,
  onReconnect,
  onForget,
  onAddHost,
}: {
  devices: ipc.DeviceInfo[];
  hosts: ipc.PairedHostView[];
  shown: ReturnType<typeof groupDevices>;
  forceHost: string | null;
  setForceHost: (id: string | null) => void;
  connected: ipc.DeviceInfo | null;
  status: ipc.CutStatus;
  aiming: boolean;
  guardUnsaved: (run: () => void) => void;
  onConnect: (info: ipc.DeviceInfo) => void;
  onDisconnect: () => void;
  onReconnect: () => void;
  onForget: (id: string, force?: boolean) => void;
  onAddHost: () => void;
}) {
  return (
    <div>
      <div style={{ fontSize: 12, color: "var(--muted-foreground)", marginBottom: 4 }}>Device</div>
      {/* Only when there is nothing at all. A paired host with no cutters attached is a
          different fact, and it has its own section saying so. */}
      {devices.length === 0 && hosts.length === 0 ? (
        <div style={{ fontSize: 12, color: "var(--muted-foreground)" }}>
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
              {section.address ? <span style={{ color: "var(--muted-foreground)" }}>{section.address}</span> : null}
              {section.stale ? <span style={{ color: "var(--muted-foreground)" }}>last known</span> : null}
              <div style={{ flex: 1 }} />
              {hosts.some((h) => h.id === section.hostId) ? (
                <button
                  aria-label={`Forget ${section.title}`}
                  style={btn}
                  onClick={() => onForget(section.hostId!)}
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
              desktop that could stop it. Shown only after the plain Forget was refused —
              hence the explicit `null` check. The local section's `hostId` is `null` too, so
              bare equality also matched it with nothing refused: a desktop that had never
              paired a host carried this warning over its local cutters, and that
              `Discard anyway` called `forget_host` for no host (#265). */}
          {forceHost !== null && forceHost === section.hostId ? (
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
                onClick={() => onForget(section.hostId!, true)}
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
                      // One aim change at a time. Two in flight would have their replies install
                      // in whatever order they land, and the first to finish would clear
                      // `aiming` while the other was still moving the aim (CodeRabbit on
                      // PR #264) — which is also what `DeviceManager` refuses as Busy.
                      disabled={aiming}
                      // Only the disconnect is guarded: a reconnect re-opens the same cutter's
                      // transport, so the draft it is editing is still that machine's.
                      onClick={() =>
                        control.verb === "reconnect" ? onReconnect() : guardUnsaved(onDisconnect)
                      }
                    >
                      {control.label}
                    </button>
                  ) : null}
                </>
              ) : (
                <button style={btn} disabled={aiming} onClick={() => guardUnsaved(() => onConnect(d))}>
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
      <button style={btn} onClick={onAddHost}>
        Add a Cut Host…
      </button>
    </div>
  );
}
