// SPDX-License-Identifier: GPL-3.0-or-later
import { useState } from "react";
import * as ipc from "../../ipc";
import type { JobSnapshot } from "../job/model";
import { toCutRequest } from "../viewmodel";
import type { InstalledPlan } from "./useCutPlan";

/** Start cut for the open sheet. Resume, cancel and confirm live on the job panel: this hook
 *  unmounts with the sheet, and a running job has to outlast it. */
export function useCutDispatch({
  connected,
  plan,
  replanning,
  weedSettled,
  canSend,
  onStalePlan,
  onError,
  onAccepted,
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
  /** Only a cut the host actually started. A duplicate keeps the sheet, and the passes it
   *  already remembered. A refusal never gets here. */
  onAccepted?: (snapshot: JobSnapshot) => void;
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
      .then((started) => {
        setAlreadyAccepted(started.duplicate);
        if (started.duplicate || plan === null) return;
        onAccepted?.({
          grouping: plan.grouping,
          // Disabled rows stay. The worker never sees them, and dropping them here would make
          // its enabled-pass index point at the wrong vinyl.
          passes: plan.rows.map((row) => ({ key: row.key, shapeCount: row.shapeCount, enabled: row.enabled })),
        });
      })
      .catch((e) => {
        // The open sheet already says the design changed and offers Replan. Toasting the same
        // refusal repeats that banner and covers the summary beside it.
        if (ipc.ipcErrorCode(e) === "stale_plan") {
          onStalePlan();
          return;
        }
        onError(ipc.ipcErrorMessage(e));
      })
      .finally(() => setCutInFlight(false));
  };

  return { alreadyAccepted, cutInFlight, startCut };
}
