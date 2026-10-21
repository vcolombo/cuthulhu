// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useState } from "react";
import * as ipc from "../ipc";

/**
 * The cutter list for the pill and the Devices tab. The cut dialog still polls while it is open
 * (phase 6 moves that interval here); this one reads on mount and when a caller asks, so a desktop
 * with no Cut Host does not walk USB twice a second.
 */
export function useDevices() {
  const [devices, setDevices] = useState<ipc.DeviceInfo[]>([]);
  const [hosts, setHosts] = useState<ipc.PairedHostView[]>([]);
  const [listError, setListError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    return Promise.all([ipc.listDevices(), ipc.listHosts()])
      .then(([nextDevices, nextHosts]) => {
        setDevices(nextDevices);
        setHosts(nextHosts);
        setListError(null);
      })
      // Shown on the Devices tab. Toasting it as well duplicates the dialog's own line, and a
      // getByText for that line then matches twice.
      .catch((e) => setListError(ipc.ipcErrorMessage(e)));
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { devices, hosts, listError, refresh };
}
