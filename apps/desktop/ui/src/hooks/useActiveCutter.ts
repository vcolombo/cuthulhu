// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { MachineProfile } from "../App";
import * as ipc from "../ipc";
import { sameCutter } from "../hosts/deviceList";

type Args = {
  profileId: string | null;
  shapeCount: number;
  machines: MachineProfile[];
  /** Writes the document machine. The only other writer of that id. */
  designFor: (machineId: string) => Promise<boolean>;
  refreshDeviceState: () => Promise<void>;
  onError: (message: string) => void;
};

/**
 * The one place a cutter becomes the connected device, and the one place that then offers to
 * retarget the document. The pill, the status bar and Send all read `device` from here.
 */
export function useActiveCutter({ profileId, shapeCount, machines, designFor, refreshDeviceState, onError }: Args) {
  const [device, setDevice] = useState<ipc.DeviceInfo | null>(null);
  const deviceRef = useRef<ipc.DeviceInfo | null>(null);
  // Aiming at a host releases the local manager, which emits Disconnected after this command
  // has already stored the host. That event is the local transport, not the aim (Bugbot on #313).
  const pendingHostReleases = useRef(0);
  const pickQueue = useRef<Promise<unknown>>(Promise.resolve());
  const refreshRef = useRef(refreshDeviceState);
  refreshRef.current = refreshDeviceState;

  const setAimed = (next: ipc.DeviceInfo | null) => {
    deviceRef.current = next;
    setDevice(next);
  };

  useEffect(() => {
    ipc.getConnectedDevice().then(setAimed).catch((e) => onError(ipc.ipcErrorMessage(e)));
  }, [onError]);

  // Same tick as the status listener: a follow-up read would still show the cutter that just
  // dropped, and the next Connect would aim at the other one (the mid-pause reconnect).
  // A polled Disconnected status does not come through here, so a host blip does not forget the aim.
  useEffect(() => {
    const unlisten = listen<ipc.DeviceEvent>("device-event", (event) => {
      if (event.payload.status.phase !== "Disconnected") return;
      if (pendingHostReleases.current > 0) {
        pendingHostReleases.current -= 1;
        // The event carried the local manager's Disconnected. The aim is the host, whose
        // status is what get_device_state returns once the command has finished.
        void refreshRef.current();
        return;
      }
      setAimed(null);
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  const nameOf = useCallback(
    (id: string) => machines.find((machine) => machine.id === id)?.name ?? id,
    [machines],
  );

  const aim = useCallback(
    async (info: ipc.DeviceInfo): Promise<boolean> => {
      if (deviceRef.current === null || !sameCutter(deviceRef.current, info)) {
        // Counted before the call: the release event can arrive while connect is still awaited,
        // and a failure is the busy refusal, which emits nothing.
        const releasingLocal = info.host !== null;
        if (releasingLocal) pendingHostReleases.current += 1;
        try {
          await ipc.connectDevice(info);
        } catch (e) {
          if (releasingLocal) pendingHostReleases.current = Math.max(0, pendingHostReleases.current - 1);
          throw e;
        }
        setAimed(info);
        await refreshDeviceState();
      }
      if (info.machine_id === profileId) return false;
      // A document that was never aimed at a machine takes the cutter's. Confirming there would
      // refuse every first connect: the test runner dismisses confirm, and so would a mis-click.
      if (profileId !== null && shapeCount > 0) {
        const ok = window.confirm(
          `This design is set up for ${nameOf(profileId)}. Switch it to ${nameOf(info.machine_id)}? The page size changes with the machine.`,
        );
        if (!ok) return false;
      }
      // True only when the document actually changed, so an open plan can be rebuilt against it.
      return designFor(info.machine_id);
    },
    [profileId, shapeCount, nameOf, designFor, refreshDeviceState],
  );
  const aimRef = useRef(aim);
  aimRef.current = aim;

  // One aim at a time. Two clicks otherwise publish whichever connect returns last.
  const pick = useCallback((info: ipc.DeviceInfo): Promise<boolean> => {
    const job = pickQueue.current.then(
      () => aimRef.current(info),
      () => aimRef.current(info),
    );
    pickQueue.current = job.then(
      () => undefined,
      () => undefined,
    );
    return job;
  }, []);

  const disconnect = useCallback(async () => {
    await ipc.disconnectDevice();
    setAimed(null);
    await refreshDeviceState();
  }, [refreshDeviceState]);

  const reconnect = useCallback(async () => {
    await ipc.reconnectDevice();
    await refreshDeviceState();
  }, [refreshDeviceState]);

  return { device, profileId, pick, disconnect, reconnect };
}
