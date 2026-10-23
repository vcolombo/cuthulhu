// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useState } from "react";
import { Cpu, RefreshCw } from "lucide-react";
import type { MachineProfile } from "../App";
import { Button } from "../components/ui/button";
import { JobButtons } from "../cut/job/JobButtons";
import { OfflineBanner } from "../cut/job/OfflineBanner";
import { passLive, passPercent, percentLabel } from "../cut/job/model";
import { TONE_BADGE } from "../cut/send/chrome";
import { HostPairing } from "../cut/send/StepMachine";
import {
  connectedControl,
  deviceBadge,
  forgetFrom,
  lastSeenLabel,
  protocolSide,
  protocolUpdateUrl,
  sameCutter,
  UPDATE_NEEDED_BADGE,
} from "../hosts/deviceList";
import * as ipc from "../ipc";
import { cn } from "../lib/cn";

type Props = {
  devices: ipc.DeviceInfo[];
  hosts: ipc.PairedHostView[];
  statuses: ipc.CutterStatusView[];
  stale: boolean;
  active: ipc.DeviceInfo | null;
  machines: MachineProfile[];
  listError: string | null;
  /** The job panel is showing the aimed cutter's cut. */
  tracking: boolean;
  status: ipc.CutStatus;
  offlineSeconds: number;
  onPick: (info: ipc.DeviceInfo) => void;
  onDisconnect: () => void;
  onRefresh: () => void;
  onViewJob: () => void;
  onJobCancel: () => void;
  onJobResume: () => void;
  onJobConfirm: () => void;
  onError: (message: string) => void;
};

const UNAVAILABLE = { label: "Unavailable", tone: "gone" as const };

/**
 * Every cutter, including the ones the aim is not on.
 *
 * A host that cannot be reached keeps the cutters from the last reading that worked, inside one
 * card. A cutter with a job still on it stays frozen at that reading; the others say they are
 * unavailable. The raw host sentence stays on the card: a protocol mismatch is that sentence,
 * and the chip and the update link are how it is acted on.
 */
export function DevicesTab({
  devices,
  hosts,
  statuses,
  stale,
  active,
  machines,
  listError,
  tracking,
  status,
  offlineSeconds,
  onPick,
  onDisconnect,
  onRefresh,
  onViewJob,
  onJobCancel,
  onJobResume,
  onJobConfirm,
  onError,
}: Props) {
  const [pairing, setPairing] = useState(false);
  const [forceHost, setForceHost] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const offline = hosts.some((host) => host.unreachable !== null);

  useEffect(() => {
    if (!offline) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [offline]);

  const nameOf = (id: string) => machines.find((machine) => machine.id === id)?.name ?? id;

  const forget = (id: string, force = false) => {
    forgetFrom(hosts, id, ipc.forgetHost, force).then((result) => {
      setForceHost(result.forceable ? id : null);
      if (result.message === null) onRefresh();
      else onError(result.message);
    });
  };

  const reconnect = (info: ipc.DeviceInfo) => {
    ipc.reconnectDevice({ host: info.host, instanceId: info.instance_id })
      .then(() => onRefresh())
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  const local = devices.filter((device) => device.host === null);
  const paired = new Set(hosts.map((host) => host.id));
  const orphanIds = [...new Set(devices.flatMap((device) => (
    device.host !== null && !paired.has(device.host) ? [device.host] : []
  )))];
  const empty = devices.length === 0 && hosts.length === 0;

  return (
    <div className="flex flex-col gap-3 p-3">
      <div className="flex items-center gap-2">
        <h2 className="flex flex-1 items-center gap-1.5 text-sm font-medium text-foreground">
          <Cpu className="size-3.5" aria-hidden />
          Cutters
        </h2>
        <Button variant="ghost" size="sm" aria-label="Refresh cutters" onClick={onRefresh}>
          <RefreshCw aria-hidden />
          Refresh
        </Button>
        <Button variant="secondary" size="sm" onClick={() => setPairing(true)}>Add cutter</Button>
      </div>

      {stale ? (
        <p className="m-0 text-xs text-muted-foreground" data-testid="devices-stale">
          These are the last values from the last reading that worked.
        </p>
      ) : null}
      {listError !== null ? (
        <details className="text-xs text-muted-foreground">
          <summary>Details</summary>
          <p className="m-0 mt-1 text-destructive">{listError}</p>
        </details>
      ) : null}

      {empty ? (
        <p className="m-0 text-sm text-foreground" data-testid="devices-body">No cutters yet</p>
      ) : null}

      {local.length > 0 ? (
        <section className="space-y-2">
          <h3 className="m-0 text-xs font-medium text-muted-foreground">This computer · USB</h3>
          <ul className="m-0 flex list-none flex-col gap-2 p-0">
            {local.map((info) => (
              <CutterCard
                key={`local:${info.instance_id}`}
                info={info}
                model={nameOf(info.machine_id)}
                where="USB on this computer"
                active={active}
                tracking={tracking}
                jobStatus={status}
                cached={null}
                offline={false}
                protocol={null}
                onPick={onPick}
                onDisconnect={onDisconnect}
                onReconnect={() => reconnect(info)}
                onViewJob={onViewJob}
                onJobCancel={onJobCancel}
                onJobResume={onJobResume}
                onJobConfirm={onJobConfirm}
                onDirectCancel={() => {
                  void ipc.cancelCut({ host: info.host, instanceId: info.instance_id }).then(onRefresh).catch((e) => onError(ipc.ipcErrorMessage(e)));
                }}
                onDirectResume={() => {
                  void ipc.resumeCut({ host: info.host, instanceId: info.instance_id }).then(onRefresh).catch((e) => onError(ipc.ipcErrorMessage(e)));
                }}
                onDirectConfirm={() => {
                  void ipc.confirmPassDone({ host: info.host, instanceId: info.instance_id }).then(onRefresh).catch((e) => onError(ipc.ipcErrorMessage(e)));
                }}
              />
            ))}
          </ul>
        </section>
      ) : null}

      {hosts.map((host) => {
        const side = protocolSide(host.protocol_side);
        const down = host.unreachable !== null;
        const rows = devices.filter((device) => device.host === host.id);
        const seconds = down ? secondsSince(host, now, offlineSeconds, active) : 0;
        const jobHere = rows.some((info) => cutterIsBusy(statusOf(info, active, tracking, status, statuses)));
        return (
          <section key={host.id} className={cn("space-y-2 rounded-lg border p-3", down ? "border-warning bg-warning/10" : "border-border bg-card")}>
            {down ? (
              <div className="space-y-1">
                <h3 className="m-0 text-sm font-medium text-foreground" data-testid="devices-body">Can't reach {host.name}</h3>
                <p className="m-0 text-xs text-muted-foreground" data-testid="devices-muted">{lastSeenLabel(host.last_seen, now)}</p>
                <p className="m-0 wrap-anywhere text-xs text-foreground">{host.address}</p>
                <p className="m-0 text-sm text-foreground" data-testid="devices-warning">{host.unreachable}</p>
                {side !== null ? (
                  <a
                    href={protocolUpdateUrl(side)}
                    target="_blank"
                    rel="noreferrer"
                    className="text-xs font-medium text-accent-ink underline-offset-2 hover:underline"
                  >
                    How to update
                  </a>
                ) : null}
                {jobHere ? <OfflineBanner host={{ ...host, unreachable: host.unreachable ?? "" }} seconds={seconds} /> : null}
              </div>
            ) : (
              <div className="space-y-1">
                <h3 className="m-0 text-sm font-medium text-foreground">{host.name} · Pi Cut Host</h3>
                <p className="m-0 text-xs text-muted-foreground">{host.address}</p>
              </div>
            )}
            <div className="flex flex-wrap gap-2">
              {down ? (
                <Button variant="secondary" size="sm" aria-label={`Retry ${host.name}`} onClick={onRefresh}>Retry</Button>
              ) : null}
              <Button variant="ghost" size="sm" aria-label={`Forget ${host.name}`} onClick={() => forget(host.id)}>Forget</Button>
            </div>
            {forceHost === host.id ? (
              <div className="flex flex-wrap items-center gap-2 text-xs text-destructive">
                <span>
                  Forgetting this Cut Host discards the credentials this desktop needs to cancel,
                  resume or confirm a cut on it. If one is running, this desktop will not be able
                  to stop it.
                </span>
                <Button variant="secondary" size="sm" aria-label={`Discard ${host.name} anyway`} onClick={() => forget(host.id, true)}>
                  Discard anyway
                </Button>
                <Button variant="ghost" size="sm" aria-label={`Keep ${host.name}`} onClick={() => setForceHost(null)}>
                  Keep it
                </Button>
              </div>
            ) : null}
            {rows.length === 0 ? (
              <p className="m-0 text-xs text-muted-foreground">No cutters are attached to it yet.</p>
            ) : (
              <ul className="m-0 flex list-none flex-col gap-2 p-0">
                {rows.map((info) => (
                  <CutterCard
                    key={`${host.id}:${info.instance_id}`}
                    info={info}
                    model={nameOf(info.machine_id)}
                    where={host.name}
                    active={active}
                    tracking={tracking}
                    jobStatus={status}
                    cached={statuses.find((row) => sameCutter(row, info))?.status ?? null}
                    offline={down}
                    protocol={side}
                    onPick={onPick}
                    onDisconnect={onDisconnect}
                    onReconnect={() => reconnect(info)}
                    onViewJob={onViewJob}
                    onJobCancel={onJobCancel}
                    onJobResume={onJobResume}
                    onJobConfirm={onJobConfirm}
                    onDirectCancel={() => {
                      void ipc.cancelCut({ host: info.host, instanceId: info.instance_id }).then(onRefresh).catch((e) => onError(ipc.ipcErrorMessage(e)));
                    }}
                    onDirectResume={() => {
                      void ipc.resumeCut({ host: info.host, instanceId: info.instance_id }).then(onRefresh).catch((e) => onError(ipc.ipcErrorMessage(e)));
                    }}
                    onDirectConfirm={() => {
                      void ipc.confirmPassDone({ host: info.host, instanceId: info.instance_id }).then(onRefresh).catch((e) => onError(ipc.ipcErrorMessage(e)));
                    }}
                  />
                ))}
              </ul>
            )}
          </section>
        );
      })}

      {orphanIds.map((id) => (
        <section key={id} className="space-y-2">
          <h3 className="m-0 text-xs font-medium text-muted-foreground">{id}</h3>
          <p className="m-0 text-xs text-destructive">this Cut Host is not paired with this computer</p>
        </section>
      ))}

      <HostPairing open={pairing} onPaired={() => { setPairing(false); onRefresh(); }} onClose={() => setPairing(false)} />
    </div>
  );
}

function secondsSince(host: ipc.PairedHostView, now: number, jobSeconds: number, active: ipc.DeviceInfo | null): number {
  if (active?.host === host.id && jobSeconds > 0) return jobSeconds;
  if (host.last_seen == null) return jobSeconds;
  return Math.max(0, Math.round((now - host.last_seen) / 1000));
}

function cutterIsBusy(row: ipc.CutStatus | null): boolean {
  if (row === null) return false;
  return row.actions.cancel || row.actions.resume || row.actions.confirm;
}

function statusOf(
  info: ipc.DeviceInfo,
  active: ipc.DeviceInfo | null,
  tracking: boolean,
  jobStatus: ipc.CutStatus,
  statuses: ipc.CutterStatusView[],
): ipc.CutStatus | null {
  if (sameCutter(active, info) && tracking) return jobStatus;
  return statuses.find((row) => sameCutter(row, info))?.status ?? null;
}

function CutterCard({
  info,
  model,
  where,
  active,
  tracking,
  jobStatus,
  cached,
  offline,
  protocol,
  onPick,
  onDisconnect,
  onReconnect,
  onViewJob,
  onJobCancel,
  onJobResume,
  onJobConfirm,
  onDirectCancel,
  onDirectResume,
  onDirectConfirm,
}: {
  info: ipc.DeviceInfo;
  model: string;
  where: string;
  active: ipc.DeviceInfo | null;
  tracking: boolean;
  jobStatus: ipc.CutStatus;
  cached: ipc.CutStatus | null;
  offline: boolean;
  protocol: "host" | "desktop" | null;
  onPick: (info: ipc.DeviceInfo) => void;
  onDisconnect: () => void;
  onReconnect: () => void;
  onViewJob: () => void;
  onJobCancel: () => void;
  onJobResume: () => void;
  onJobConfirm: () => void;
  onDirectCancel: () => void;
  onDirectResume: () => void;
  onDirectConfirm: () => void;
}) {
  const aimed = sameCutter(active, info);
  const live = aimed && tracking ? jobStatus : cached;
  const busy = cutterIsBusy(live);
  const badge = protocol !== null
    ? UPDATE_NEEDED_BADGE
    : offline && !busy
      ? UNAVAILABLE
      : deviceBadge(live);
  const control = connectedControl(live, info.host !== null);
  const showControl = control !== null && (control.verb === "reconnect" || aimed);
  const jobControls = aimed && tracking;
  const pct = live === null ? null : passPercent(live.sent);
  const frozen = offline || (live !== null && !passLive(live.phase, pct, offline) && busy);

  return (
    <li className="rounded-lg border border-border bg-card p-3">
      <div className="flex items-start gap-2">
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium text-foreground">{model}</span>
          <span className="block text-xs text-muted-foreground">{where}</span>
        </span>
        <span
          data-testid="device-badge"
          data-tone={badge.tone}
          className={cn("shrink-0 rounded-full border px-2 py-0.5 text-2xs font-medium", TONE_BADGE[badge.tone])}
        >
          {badge.label}
        </span>
      </div>
      {!aimed && badge.tone === "busy" ? (
        <p className="m-0 mt-1 text-xs text-muted-foreground">This cutter is in the middle of a job.</p>
      ) : null}
      {busy && pct !== null && live !== null ? (
        <div className="mt-2">
          <div
            data-testid="pass-bar"
            role="progressbar"
            aria-label="This pass"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
            className="h-2 overflow-hidden rounded-full bg-subtle"
          >
            <div
              className={frozen ? "h-full bg-primary" : "progress-live h-full bg-primary"}
              style={{ width: `${pct}%` }}
            />
          </div>
          <p data-testid="pass-progress" className="mt-1 text-xs text-muted-foreground">{percentLabel(pct)}</p>
        </div>
      ) : null}
      <div className="mt-2 flex flex-wrap gap-2">
        <Button
          variant="secondary"
          size="sm"
          disabled={aimed || (!aimed && badge.tone === "busy")}
          onClick={() => onPick(info)}
        >
          {aimed ? "Active" : "Set as active"}
        </Button>
        {aimed && tracking ? (
          <Button variant="ghost" size="sm" onClick={onViewJob}>Open job</Button>
        ) : null}
        {showControl ? (
          <Button
            variant="secondary"
            size="sm"
            aria-label={`${control.label} ${info.instance_id}`}
            onClick={() => (control.verb === "reconnect" ? onReconnect() : onDisconnect())}
          >
            {control.label}
          </Button>
        ) : null}
        {busy && live !== null ? (
          <JobButtons
            actions={live.actions}
            place="panel"
            offline={offline}
            onCancel={jobControls ? onJobCancel : onDirectCancel}
            onResume={jobControls ? onJobResume : onDirectResume}
            onConfirm={jobControls ? onJobConfirm : onDirectConfirm}
          />
        ) : null}
      </div>
    </li>
  );
}
