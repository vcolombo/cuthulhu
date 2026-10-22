// SPDX-License-Identifier: GPL-3.0-or-later
import { useState } from "react";
import * as ipc from "../../ipc";
import { toCutRequest } from "../viewmodel";
import type { InstalledPlan } from "./useCutPlan";

/** Cut, cancel, resume and confirm for the open sheet. Phase 5's job panel reads the same
 *  three: the in-flight latch, the "already accepted" answer, and the four actions. */
export function useCutDispatch({
  connected,
  plan,
  replanning,
  weedSettled,
  canSend,
  onStalePlan,
  onError,
  refreshDeviceState,
}: {
  connected: ipc.DeviceInfo | null;
  plan: InstalledPlan | null;
  replanning: boolean;
  weedSettled: boolean;
  /** Blank or out-of-range speed and force never leave the sheet. The button is disabled
   *  for the same reason; this is the check that still holds if the click arrives anyway. */
  canSend: () => boolean;
  onStalePlan: () => void;
  onError: (msg: string) => void;
  refreshDeviceState: () => Promise<void>;
}) {
  /** Set when the Cut Host answered that it had already accepted this dispatch, so nothing new
   *  started. Cleared by the next press of Cut, which is the thing that makes it untrue. */
  const [alreadyAccepted, setAlreadyAccepted] = useState(false);
  /** A cut request is out. The backend reserves one dispatch id per Job, so a second click is
   *  safe — it joins the first rather than starting a second Job — but it would come back
   *  "already accepted", which is a confusing thing to say about a double-click. */
  const [cutInFlight, setCutInFlight] = useState(false);

  const startCut = () => {
    // `replanning` guards the window a mode change opens: the rows on screen still belong to
    // the previous grouping until the new plan installs, and sending them under the new one
    // would cut whatever that mode happens to key the same way.
    if (!connected || plan === null || replanning || !weedSettled || !canSend()) return;
    const request = toCutRequest(connected.instance_id, plan.revision, plan.grouping, plan.weed, plan.rows);
    setAlreadyAccepted(false);
    setCutInFlight(true);
    ipc
      .cut(request)
      // Not an error, and not a success worth saying nothing about: the host recognised this
      // dispatch and started nothing. Told apart, because a cutter that is not moving looks the
      // same either way and only the host knows which happened.
      .then((started) => setAlreadyAccepted(started.duplicate))
      .catch((e) => {
        const code = ipc.ipcErrorCode(e);
        if (code === "stale_plan") onStalePlan();
        onError(ipc.ipcErrorMessage(e));
      })
      .finally(() => setCutInFlight(false));
  };

  const resume = () => {
    ipc
      .resumeCut()
      .then(() => refreshDeviceState())
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  const cancel = () => {
    ipc
      .cancelCut()
      .then(() => refreshDeviceState())
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  const confirmPassDone = () => {
    ipc
      .confirmPassDone()
      .then(() => refreshDeviceState())
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  };

  return { alreadyAccepted, cutInFlight, startCut, resume, cancel, confirmPassDone };
}
