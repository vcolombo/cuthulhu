// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { MachineProfile } from "../App";
import * as ipc from "../ipc";

type Args = {
  profileId: string | null;
  shapeCount: number;
  machines: MachineProfile[];
  /** Writes the document machine. The only other writer of that id. */
  designFor: (machineId: string) => Promise<boolean>;
  refreshDeviceState: () => Promise<void>;
  onError: (message: string) => void;
};

function sameCutter(a: ipc.DeviceInfo, b: ipc.DeviceInfo): boolean {
  return a.instance_id === b.instance_id && a.host === b.host;
}

/**
 * The one place a cutter becomes the connected device, and the one place that then offers to
 * retarget the document. The pill, the status bar and Send all read `device` from here.
 */
export function useActiveCutter({ profileId, shapeCount, machines, designFor, refreshDeviceState, onError }: Args) {
  const [device, setDevice] = useState<ipc.DeviceInfo | null>(null);

  useEffect(() => {
    ipc.getConnectedDevice().then(setDevice).catch((e) => onError(ipc.ipcErrorMessage(e)));
  }, [onError]);

  // Same tick as the status listener: a follow-up read would still show the cutter that just
  // dropped, and the next Connect would aim at the other one (the mid-pause reconnect).
  // A polled Disconnected status does not come through here, so a host blip does not forget the aim.
  useEffect(() => {
    const unlisten = listen<ipc.DeviceEvent>("device-event", (event) => {
      if (event.payload.status.phase === "Disconnected") setDevice(null);
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  const nameOf = useCallback(
    (id: string) => machines.find((machine) => machine.id === id)?.name ?? id,
    [machines],
  );

  const pick = useCallback(
    async (info: ipc.DeviceInfo): Promise<boolean> => {
      if (device === null || !sameCutter(device, info)) {
        await ipc.connectDevice(info);
        setDevice(info);
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
      await designFor(info.machine_id);
      // True only when the document actually changed, so an open plan can be rebuilt against it.
      return true;
    },
    [device, profileId, shapeCount, nameOf, designFor, refreshDeviceState],
  );

  const disconnect = useCallback(async () => {
    await ipc.disconnectDevice();
    setDevice(null);
    await refreshDeviceState();
  }, [refreshDeviceState]);

  const reconnect = useCallback(async () => {
    await ipc.reconnectDevice();
    await refreshDeviceState();
  }, [refreshDeviceState]);

  return { device, profileId, pick, disconnect, reconnect };
}
