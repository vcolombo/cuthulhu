// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useState } from "react";
import type { MachineProfile } from "../../App";
import * as ipc from "../../ipc";
import { Laptop, Router } from "lucide-react";
import { Button } from "../../components/ui/button";
import { cn } from "../../lib/cn";
import { connectedControl, deviceBadge, forgetFrom, groupDevices, protocolSide, protocolUpdateUrl, sameCutter, staleSection, UPDATE_NEEDED_BADGE } from "../../hosts/deviceList";
import { PairHostDialog } from "../../hosts/PairHostDialog";
import { TONE_BADGE } from "./chrome";

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
  // request/reply connection. This interval covers the open sheet. `useJob` keeps asking
  // after the sheet closes, because the job panel has to move once this unmounts.
  //
  // With no host there is no gap to fill — a local cutter's status is pushed by the
  // device-event listener in `useJob` — and the cost is not nothing: `list_devices` resolves
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

  return { devices, hosts, shown, forceHost, setForceHost, pairing, setPairing, forget, paired, reconnect, refreshList };
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
  machines,
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
  onRetry,
}: {
  devices: ipc.DeviceInfo[];
  hosts: ipc.PairedHostView[];
  machines: MachineProfile[];
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
  onRetry: () => void;
}) {
  return (
    <div className="space-y-4">
      {/* Only when there is nothing at all. A paired host with no cutters attached is a
          different fact, and it has its own section saying so. */}
      {devices.length === 0 && hosts.length === 0 ? (
        <div className="text-sm text-muted-foreground">
          No devices found — connect a cutter and reopen this dialog.
        </div>
      ) : null}
      {shown.map((section) => {
        const side = protocolSide(section.unreachable);
        return (
        <div key={section.hostId ?? "local"} className="space-y-2">
          {/* The local section's header is suppressed when it is the only one, so a user with
              no Cut Host sees the flat list this dialog has always shown — but not when the
              read failed. The "last known" marker lives in this header, so suppressing it
              left the reason below as an unlabelled red line of Rust prose under "Device",
              with nothing saying which list it was about or that the rows had stopped
              moving. A failure the user cannot place is worse than a heading they did not
              need. */}
          {section.hostId === null && hosts.length === 0 && !section.stale ? null : (
            <div className="flex items-center gap-2 text-sm">
              <strong>{section.title}</strong>
              {section.address ? <span className="text-muted-foreground">{section.address}</span> : null}
              {section.stale ? <span className="text-muted-foreground">last known</span> : null}
              <div className="flex-1" />
              {section.unreachable ? (
                <Button variant="ghost" size="sm" aria-label={`Retry ${section.title}`} onClick={onRetry}>Retry</Button>
              ) : null}
              {hosts.some((h) => h.id === section.hostId) ? (
                <Button variant="ghost" size="sm" aria-label={`Forget ${section.title}`} onClick={() => onForget(section.hostId!)}>
                  Forget
                </Button>
              ) : null}
            </div>
          )}
          {/* An unreachable host keeps its cutters listed below this, not hidden (#42). */}
          {section.unreachable ? (
            <div className="text-sm text-destructive">
              {section.unreachable}
              {side !== null ? (
                <>
                  {" "}
                  <a
                    href={protocolUpdateUrl(side)}
                    target="_blank"
                    rel="noreferrer"
                    className="text-sm font-medium text-accent-ink underline-offset-2 hover:underline"
                  >
                    How to update
                  </a>
                </>
              ) : null}
            </div>
          ) : null}
          {/* Says what is being accepted rather than asking "are you sure". The two refusals
              this appears for differ in why — a host that could not be asked, or one whose
              cutter nothing can reach — and each states its own reason in the Rust side's
              prose, which `onError` has already put in front of the operator. What is stated
              here is the part that is the same either way and is the actual decision: the
              credentials go, and with them every route to a blade that may still be moving.
              Shown only after the plain Forget was refused — hence the explicit `null` check.
              The local section's `hostId` is `null` too, so bare equality also matched it with
              nothing refused: a desktop that had never paired a host carried this warning over
              its local cutters, and that `Discard anyway` called `forget_host` for no host
              (#265). */}
          {forceHost !== null && forceHost === section.hostId ? (
            <div className="flex items-center gap-2 text-sm text-destructive">
              <span>
                Forgetting this Cut Host discards the credentials this desktop needs to cancel,
                resume or confirm a cut on it. If one is running, this desktop will not be able
                to stop it.
              </span>
              {/* Not "Forget <name> anyway": that name contains this section's own Forget
                  button's, and a selector for one would match both. */}
              <Button variant="secondary" size="sm" aria-label={`Discard ${section.title} anyway`} onClick={() => onForget(section.hostId!, true)}>
                Discard anyway
              </Button>
              <Button variant="ghost" size="sm" aria-label={`Keep ${section.title}`} onClick={() => setForceHost(null)}>
                Keep it
              </Button>
            </div>
          ) : null}
          <div role="radiogroup" aria-label={section.title} className="space-y-2">
            {section.devices.map((d) => {
              // Only the aimed-at cutter has a status; the rest have not been asked, and
              // `null` is what says so rather than something that reads as ready. Matched on the
              // id *and* the host: two hosts wired alike hand out the same fallback id, and
              // giving one host's status to the other's row hands Cancel to the wrong machine.
              const isAimed = sameCutter(connected, d);
              const aimed = isAimed ? status : null;
              const live = deviceBadge(aimed);
              // A mismatch is not an unfetched status. The chip uses the same tones as
              // "Unknown"; the label is what the message above already asked for.
              const badge = side !== null ? UPDATE_NEEDED_BADGE : live;
              const control = connectedControl(aimed, d.host !== null);
              // A busy cutter cannot be chosen until a queue exists, and an unreachable host's
              // cutters stay listed but cannot be aimed at. The aimed cutter stays selectable:
              // Reconnect is the way back from a stop nothing confirmed.
              const blocked = section.unreachable !== null || (!isAimed && live.tone === "busy");
              const reason = live.tone === "busy" && side === null ? "This cutter is in the middle of a job." : null;
              const Icon = d.host === null ? Laptop : Router;
              const model = machines.find((machine) => machine.id === d.machine_id)?.name ?? d.machine_id;
              // The section header already names a host. The card's second line is how this
              // cutter is reached, which for a serial port that has not been confirmed is the
              // caveat that used to sit in parentheses on the raw id.
              const host = d.host === null ? null : hosts.find((item) => item.id === d.host) ?? null;
              const where = d.host === null
                ? (d.candidate ? "Unverified serial port on this computer" : "USB on this computer")
                : host
                  ? `${host.name} (${host.address})`
                  : "Cut Host";
              return (
                <div
                  key={`${d.host ?? "local"}:${d.instance_id}`}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg border bg-card p-3 text-left",
                    isAimed && !blocked ? "border-accent-ink ring-1 ring-accent-ink" : "border-border",
                    blocked && "opacity-70",
                  )}
                >
                  {/* The card is the choice. A second Connect button asked the same question
                      and went through the same aim. Disconnect and Reconnect stay beside it:
                      choosing the card again is not how a transport is dropped or reopened. */}
                  <button
                    type="button"
                    role="radio"
                    aria-checked={isAimed}
                    aria-label={model}
                    disabled={aiming || (blocked && !isAimed)}
                    onClick={() => {
                      if (isAimed || blocked) return;
                      guardUnsaved(() => onConnect(d));
                    }}
                    className="flex min-w-0 flex-1 items-center gap-3 text-left disabled:cursor-not-allowed"
                  >
                    <span
                      aria-hidden
                      className={cn(
                        "grid size-4 shrink-0 place-items-center rounded-full border",
                        isAimed && !blocked ? "border-accent-ink" : "border-input",
                      )}
                    >
                      {isAimed && !blocked ? <span className="size-2 rounded-full bg-primary" /> : null}
                    </span>
                    <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
                    <span className="min-w-0 flex-1">
                      <span className="block text-sm font-medium">{model}</span>
                      <span className="block text-xs text-muted-foreground">{where}</span>
                      {reason ? <span className="mt-1 block text-xs text-muted-foreground">{reason}</span> : null}
                    </span>
                  </button>
                  <span
                    data-testid="device-badge"
                    data-tone={badge.tone}
                    className={cn("shrink-0 rounded-full border px-2 py-0.5 text-2xs font-medium", TONE_BADGE[badge.tone])}
                  >
                    {badge.label}
                  </span>
                  {isAimed && control ? (
                    // The only way back from a stop nothing confirmed: `driver-core` refuses
                    // both a cut and a connect from that state. Withheld mid-Job — `connectedControl`.
                    <Button
                      variant="secondary"
                      size="sm"
                      aria-label={`${control.label} ${d.instance_id}`}
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
                    </Button>
                  ) : null}
                </div>
              );
            })}
          </div>
        </div>
        );
      })}
      {/* Pairing lives in the device list on purpose: someone hunting for their Pi looks
          here, and finds nothing if it lives in a settings screen. */}
      <Button variant="secondary" onClick={onAddHost}>Add a Cut Host…</Button>
    </div>
  );
}
