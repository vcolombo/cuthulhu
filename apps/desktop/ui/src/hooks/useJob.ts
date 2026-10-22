// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import * as ipc from "../ipc";

/** The worker's published status. One listener: events arrive in order on one channel. */
export function useJob(onError: (message: string) => void) {
  const [status, setStatus] = useState<ipc.CutStatus>(ipc.DISCONNECTED_STATUS);

  const refreshDeviceState = useCallback(() => {
    return ipc
      .getDeviceState()
      .then((next) => {
        setStatus(next);
      })
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  }, [onError]);

  useEffect(() => {
    const unlisten = listen<ipc.DeviceEvent>("device-event", (event) => {
      setStatus(event.payload.status);
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  return { status, refreshDeviceState };
}
