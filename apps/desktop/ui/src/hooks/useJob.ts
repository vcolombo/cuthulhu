// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as ipc from "../ipc";
import { jobIsActive, type JobSnapshot } from "../cut/job/model";

const UNREACHABLE = "the host could not be reached";

/** What the panel shows the moment a cut is accepted, before the worker's Sending
 *  event. A terminal status left on screen keeps Completed or Cut failed up, and
 *  the host poll never starts for a phase that is not active. */
const ACCEPTED_STATUS: ipc.CutStatus = {
  phase: "Sending",
  ended: null,
  actions: { cut: false, cancel: true, resume: false, confirm: false },
  pass: null,
  sent: null,
  error: null,
  jobId: 0,
};

function sameHost(a: ipc.PairedHostView | null, b: ipc.PairedHostView): boolean {
  return a !== null && a.id === b.id && a.name === b.name && a.address === b.address
    && a.unreachable === b.unreachable && a.protocol_side === b.protocol_side;
}

/**
 * The worker's published status, plus the passes this session sent.
 *
 * A Cut Host is not told to us: the sheet's poll stops when the sheet closes, and the job
 * has to keep moving after that. While the job's host is unreachable the live read is
 * `disconnected` on purpose, so it is discarded and the last good status stays on screen.
 */
export function useJob(onError: (message: string) => void) {
  const [status, setStatus] = useState<ipc.CutStatus>(ipc.DISCONNECTED_STATUS);
  const [snapshot, setSnapshot] = useState<JobSnapshot | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [hostId, setHostId] = useState<string | null>(null);
  const [offlineHost, setOfflineHost] = useState<ipc.PairedHostView | null>(null);
  const [lastStatusAt, setLastStatusAt] = useState(() => Date.now());
  const [now, setNow] = useState(() => Date.now());

  const statusRef = useRef(status);
  const snapshotRef = useRef(snapshot);
  // During render, not a layout effect. The listeners and the poll read these between
  // commit and layout, which is the window the mirror exists for.
  statusRef.current = status;
  snapshotRef.current = snapshot;
  const offlineRef = useRef(false);
  const hostIdRef = useRef<string | null>(null);
  const instanceRef = useRef<string | null>(null);
  /** Bumped when the aimed cutter changes. Host id alone misses a return to no
   *  cutter, and a same-host switch, and either one lets a stale poll land. */
  const aimGenRef = useRef(0);
  const knownHostRef = useRef<ipc.PairedHostView | null>(null);
  const cancelQueuedRef = useRef(false);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  /** The id this session accepted. Null when a remote accept reported none. */
  const acceptedJobRef = useRef<number | null>(null);
  /** When the accept reported no id, a status at or below this is the previous cut. */
  const floorRef = useRef(0);
  /** Bumped in `remember`. A read started before this accept still belongs to the
   *  previous aim, and must not land on the surface this accept just put up. */
  const acceptGenRef = useRef(0);

  const ingest = useCallback((next: ipc.CutStatus) => {
    if (offlineRef.current) return;
    // A host that is still aimed does not go idle by reporting Disconnected. That
    // read is a blip; the poll freezes the last status instead.
    if (hostIdRef.current !== null && next.phase === "Disconnected") return;
    const id = next.jobId;
    // 0 is a lifecycle report, not an older job. A smaller id is the previous cut.
    // A larger one can arrive before accept returns: that is this cut.
    if (id !== 0) {
      const accepted = acceptedJobRef.current;
      if (accepted !== null && id < accepted) return;
      if (accepted === null && id <= floorRef.current) return;
      // The first id of a cut whose accept named none. A higher id that arrives
      // while one is already accepted is the next cut, and stays above the floor
      // until `remember` claims it — raising the floor here would hide that cut.
      if (accepted === null) floorRef.current = id;
      acceptedJobRef.current = id;
    }
    statusRef.current = next;
    setStatus(next);
    setLastStatusAt(Date.now());
  }, []);

  const freeze = useCallback((host: ipc.PairedHostView) => {
    offlineRef.current = true;
    setNow(Date.now());
    setOfflineHost((prev) => (sameHost(prev, host) ? prev : host));
  }, []);

  const refreshDeviceState = useCallback((): Promise<void> => {
    const id = hostIdRef.current;
    const gen = acceptGenRef.current;
    const aimGen = aimGenRef.current;
    const watching = id !== null && snapshotRef.current !== null && (jobIsActive(statusRef.current.phase) || offlineRef.current);
    if (!watching || id === null) {
      return ipc.getDeviceState().then((next) => {
        if (acceptGenRef.current !== gen) return;
        // `aim()` starts this read, then the effect's `attach` bumps the generation.
        // Dropping it leaves the previous surface up: nothing else is polling.
        if (aimGenRef.current !== aimGen) {
          const nextGen = aimGenRef.current;
          return ipc.getDeviceState().then((fresh) => {
            if (acceptGenRef.current !== gen || aimGenRef.current !== nextGen) return;
            ingest(fresh);
          });
        }
        ingest(next);
      }).catch((e) => {
        onErrorRef.current(ipc.ipcErrorMessage(e));
      });
    }
    return Promise.allSettled([ipc.listHosts(), ipc.getDeviceState()]).then(([hostsResult, statusResult]) => {
      if (acceptGenRef.current !== gen) return;
      // The aim moved while this read was in flight. Freezing here would keep the
      // swap bar after the cutter is gone; the detach effect may already have returned.
      if (hostIdRef.current !== id) {
        cancelQueuedRef.current = false;
        offlineRef.current = false;
        setOfflineHost(null);
        const aimGen = aimGenRef.current;
        return ipc.getDeviceState().then((next) => {
          if (acceptGenRef.current !== gen || aimGenRef.current !== aimGen) return;
          ingest(next);
        }).catch((e) => {
          onErrorRef.current(ipc.ipcErrorMessage(e));
        });
      }
      // Same host, different cutter. This read was started for the previous one.
      // Read again for the cutter now aimed, or the switch keeps the old surface.
      if (aimGenRef.current !== aimGen) {
        const nextGen = aimGenRef.current;
        return ipc.getDeviceState().then((fresh) => {
          if (acceptGenRef.current !== gen || aimGenRef.current !== nextGen) return;
          ingest(fresh);
        }).catch((e) => {
          onErrorRef.current(ipc.ipcErrorMessage(e));
        });
      }
      const host = hostsResult.status === "fulfilled" ? hostsResult.value.find((item) => item.id === id) ?? null : null;
      if (host !== null) knownHostRef.current = host;
      const unreachable = hostsResult.status === "fulfilled" && (host === null || host.unreachable !== null);
      // A live read while the host is down is `disconnected`, which would wipe the bar.
      // A failed read is the same fact: keep the last status that arrived.
      const liveDrop = statusResult.status === "fulfilled" && statusResult.value.phase === "Disconnected";
      if (statusResult.status === "rejected" || unreachable || liveDrop) {
        const known = knownHostRef.current;
        const base = host ?? (known !== null && known.id === id ? known : null);
        freeze({
          id,
          name: base?.name ?? id,
          address: base?.address ?? "",
          unreachable: host?.unreachable ?? base?.unreachable ?? UNREACHABLE,
          // A returned host's null is the fact. A missing host keeps the last side.
          protocol_side: host !== null ? host.protocol_side : (base?.protocol_side ?? null),
        });
        return;
      }
      const next = statusResult.value;
      const wasQueued = cancelQueuedRef.current;
      offlineRef.current = false;
      setOfflineHost(null);
      if (wasQueued) cancelQueuedRef.current = false;
      // A cut that finished during the outage is not cancellable. Sending the queued
      // cancel would rest that job on Cancelled.
      if (wasQueued && jobIsActive(next.phase) && next.actions.cancel) {
        // `next` is the poll from before the host answered. The cancel emit is the status;
        // writing the poll over it would put Sending back on top of Cancelling.
        return ipc.cancelCut().catch((e) => {
          ingest(next);
          onErrorRef.current(ipc.ipcErrorMessage(e));
        });
      }
      ingest(next);
    });
  }, [freeze, ingest]);

  const refreshRef = useRef(refreshDeviceState);
  refreshRef.current = refreshDeviceState;

  const attach = useCallback((device: ipc.DeviceInfo | null) => {
    const next = device?.host ?? null;
    const instance = device?.instance_id ?? null;
    if (hostIdRef.current !== next || instanceRef.current !== instance) {
      aimGenRef.current += 1;
      // The queued cancel named the cutter we just left.
      cancelQueuedRef.current = false;
      // Job ids start over on each cutter. Keeping the previous floor would drop
      // a smaller id on the one now aimed, and cancel would hit that cutter while
      // the old job stayed on screen. Clearing the floor on the way out would let
      // a late event from the cutter we left back in; the detach read replaces it.
      if (instance !== null) {
        acceptedJobRef.current = null;
        floorRef.current = 0;
      }
    }
    instanceRef.current = instance;
    hostIdRef.current = next;
    setHostId((prev) => (prev === next ? prev : next));
  }, []);

  useEffect(() => {
    if (hostId !== null) return;
    if (!offlineRef.current && !jobIsActive(statusRef.current.phase)) return;
    // The aim left during a remote job. Disconnected is dropped while a host is still
    // aimed, and a poll that resolves afterwards must not be what keeps Resume on screen.
    // On mount the phase is already Disconnected and nothing is frozen, so this is a no-op.
    offlineRef.current = false;
    setOfflineHost(null);
    cancelQueuedRef.current = false;
    const aimGen = aimGenRef.current;
    void ipc.getDeviceState().then((next) => {
      if (aimGenRef.current !== aimGen) return;
      ingest(next);
    }).catch((e) => onErrorRef.current(ipc.ipcErrorMessage(e)));
  }, [hostId, ingest]);

  useEffect(() => {
    const unlisten = listen<ipc.DeviceEvent>("device-event", (event) => {
      ingest(event.payload.status);
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, [ingest]);

  const watching = hostId !== null && snapshot !== null && (jobIsActive(status.phase) || offlineHost !== null);
  useEffect(() => {
    if (!watching) return;
    let stopped = false;
    let inFlight = false;
    const tick = () => {
      if (stopped || inFlight) return;
      inFlight = true;
      void Promise.resolve(refreshRef.current()).finally(() => {
        inFlight = false;
      });
    };
    tick();
    const id = window.setInterval(tick, 1000);
    return () => {
      stopped = true;
      window.clearInterval(id);
    };
  }, [watching, hostId]);

  useEffect(() => {
    if (offlineHost === null) return;
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [offlineHost]);

  const remember = useCallback((next: JobSnapshot) => {
    acceptGenRef.current += 1;
    snapshotRef.current = next;
    setSnapshot(next);
    setDismissed(false);
    cancelQueuedRef.current = false;
    const prev = statusRef.current;
    if (next.jobId !== 0) {
      acceptedJobRef.current = next.jobId;
      floorRef.current = next.jobId;
    } else {
      // `Accepted` carries no id. An id above the floor is this cut, published
      // before the accept returned. Anything else on screen is the previous cut.
      const early = prev.jobId !== 0 && prev.jobId > floorRef.current && acceptedJobRef.current === prev.jobId;
      if (early) {
        floorRef.current = prev.jobId;
        return;
      }
      if (prev.jobId !== 0) floorRef.current = prev.jobId;
      acceptedJobRef.current = null;
    }
    // This cut can publish, including a failure, before accept returns. That status
    // is the job. Replacing it would hide the only one this cut is going to emit.
    if (next.jobId !== 0 && prev.jobId === next.jobId) return;
    const placeholder: ipc.CutStatus = { ...ACCEPTED_STATUS, jobId: next.jobId };
    statusRef.current = placeholder;
    setStatus(placeholder);
    setLastStatusAt(Date.now());
  }, []);

  const dismiss = useCallback(() => setDismissed(true), []);

  const resume = useCallback(() => {
    if (offlineRef.current) return;
    ipc.resumeCut().then(() => refreshRef.current()).catch((e) => onErrorRef.current(ipc.ipcErrorMessage(e)));
  }, []);

  const confirmPassDone = useCallback(() => {
    if (offlineRef.current) return;
    ipc.confirmPassDone().then(() => refreshRef.current()).catch((e) => onErrorRef.current(ipc.ipcErrorMessage(e)));
  }, []);

  const cancel = useCallback(() => {
    if (offlineRef.current) {
      cancelQueuedRef.current = true;
      return;
    }
    ipc.cancelCut().then(() => refreshRef.current()).catch((e) => onErrorRef.current(ipc.ipcErrorMessage(e)));
  }, []);

  const offlineSeconds = offlineHost === null ? 0 : Math.max(0, Math.floor((now - lastStatusAt) / 1000));

  return {
    status,
    refreshDeviceState,
    snapshot,
    remember,
    dismiss,
    tracking: snapshot !== null && !dismissed,
    offlineHost,
    offlineSeconds,
    resume,
    cancel,
    confirmPassDone,
    attach,
  };
}
