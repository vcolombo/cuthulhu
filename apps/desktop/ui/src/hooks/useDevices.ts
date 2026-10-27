// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";
import * as ipc from "../ipc";
import { retainOffline } from "../hosts/deviceList";

/** While the Devices tab, the Send sheet, or a job is on screen. */
export const LIVE_POLL_MS = 1000;
/** A paired host that nobody is looking at. Long enough that closing the sheet stops the 1s dial. */
export const IDLE_POLL_MS = 10_000;

export type DevicesPoll = {
  hosts: ipc.PairedHostView[];
  statuses: ipc.CutterStatusView[];
  /** The whole read failed. Rows on screen are the previous ones. */
  stale: boolean;
};

/**
 * The one poll for every paired Cut Host.
 *
 * One interval, and a tick that is still waiting is skipped rather than queued: an unreachable
 * host can take seconds to answer, and a backlog of those outlives whatever wedged it. The three
 * reads run one after another. `list_devices` is the dial; `cutter_statuses` only reads what that
 * dial stored, so two of them at once would be a second client on a daemon that caps them at
 * eight (#103). With no host paired there is no interval at all — a local cutter is pushed, and
 * `list_devices` walks USB.
 */
export function useDevices(live: boolean, onPolled?: (poll: DevicesPoll) => void) {
  const [devices, setDevices] = useState<ipc.DeviceInfo[]>([]);
  const [hosts, setHosts] = useState<ipc.PairedHostView[]>([]);
  const [statuses, setStatuses] = useState<ipc.CutterStatusView[]>([]);
  const [stale, setStale] = useState(false);
  const [listError, setListError] = useState<string | null>(null);

  const devicesRef = useRef(devices);
  const hostsRef = useRef(hosts);
  const statusesRef = useRef(statuses);
  devicesRef.current = devices;
  hostsRef.current = hosts;
  statusesRef.current = statuses;
  const onPolledRef = useRef(onPolled);
  onPolledRef.current = onPolled;
  const inFlight = useRef(false);

  const pull = useCallback(async () => {
    try {
      const nextDevices = await ipc.listDevices();
      const nextHosts = await ipc.listHosts();
      const nextStatuses = await ipc.cutterStatuses();
      const retained = retainOffline(
        devicesRef.current,
        statusesRef.current,
        nextDevices,
        nextHosts,
        nextStatuses,
      );
      devicesRef.current = retained.devices;
      hostsRef.current = nextHosts;
      statusesRef.current = retained.statuses;
      setDevices(retained.devices);
      setHosts(nextHosts);
      setStatuses(retained.statuses);
      setStale(false);
      setListError(null);
      onPolledRef.current?.({ hosts: nextHosts, statuses: retained.statuses, stale: false });
    } catch (e) {
      setStale(true);
      setListError(ipc.ipcErrorMessage(e));
      onPolledRef.current?.({
        hosts: hostsRef.current,
        statuses: statusesRef.current,
        stale: true,
      });
    }
  }, []);

  const refresh = useCallback(() => {
    if (inFlight.current) return Promise.resolve();
    inFlight.current = true;
    return pull().finally(() => {
      inFlight.current = false;
    });
  }, [pull]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const hostCount = hosts.length;
  useEffect(() => {
    if (hostCount === 0) return;
    let stopped = false;
    const tick = () => {
      if (stopped || inFlight.current) return;
      inFlight.current = true;
      void pull().finally(() => {
        inFlight.current = false;
      });
    };
    const id = window.setInterval(tick, live ? LIVE_POLL_MS : IDLE_POLL_MS);
    return () => {
      stopped = true;
      window.clearInterval(id);
    };
    // Length, not the array: replacing `hosts` with an equal list every tick would rebuild it.
  }, [hostCount, live, pull]);

  return { devices, hosts, statuses, stale, listError, refresh };
}
