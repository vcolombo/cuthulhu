// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as ipc from "../ipc";
import { jobIsActive, type JobSnapshot } from "../cut/job/model";

const UNREACHABLE = "the host could not be reached";

function sameHost(a: ipc.PairedHostView | null, b: ipc.PairedHostView): boolean {
  return a !== null && a.id === b.id && a.name === b.name && a.address === b.address && a.unreachable === b.unreachable;
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
  statusRef.current = status;
  const snapshotRef = useRef(snapshot);
  snapshotRef.current = snapshot;
  const offlineRef = useRef(false);
  const hostIdRef = useRef<string | null>(null);
  const knownHostRef = useRef<ipc.PairedHostView | null>(null);
  const cancelQueuedRef = useRef(false);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const ingest = useCallback((next: ipc.CutStatus) => {
    if (offlineRef.current) return;
    // A host blip's live read is Disconnected, and it is not the job. The poll freezes
    // instead; applying it here would drop the pass the operator is still watching.
    if (
      hostIdRef.current !== null
      && snapshotRef.current !== null
      && jobIsActive(statusRef.current.phase)
      && next.phase === "Disconnected"
    ) {
      return;
    }
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
    const watching = id !== null && snapshotRef.current !== null && (jobIsActive(statusRef.current.phase) || offlineRef.current);
    if (!watching || id === null) {
      return ipc.getDeviceState().then(ingest).catch((e) => {
        onErrorRef.current(ipc.ipcErrorMessage(e));
      });
    }
    return Promise.allSettled([ipc.listHosts(), ipc.getDeviceState()]).then(([hostsResult, statusResult]) => {
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
        });
        return;
      }
      const next = statusResult.value;
      const wasQueued = cancelQueuedRef.current;
      offlineRef.current = false;
      setOfflineHost(null);
      if (wasQueued) {
        cancelQueuedRef.current = false;
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
    hostIdRef.current = next;
    setHostId((prev) => (prev === next ? prev : next));
  }, []);

  useEffect(() => {
    if (hostId !== null || !offlineRef.current) return;
    // The aim left while we were frozen, so the Disconnected event was discarded with the
    // rest. Ask once more now that there is no host to protect the last good status for.
    offlineRef.current = false;
    setOfflineHost(null);
    void ipc.getDeviceState().then(ingest).catch((e) => onErrorRef.current(ipc.ipcErrorMessage(e)));
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
    setSnapshot(next);
    setDismissed(false);
    cancelQueuedRef.current = false;
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
