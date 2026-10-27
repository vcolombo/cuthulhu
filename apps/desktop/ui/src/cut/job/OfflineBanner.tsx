// SPDX-License-Identifier: GPL-3.0-or-later
import type { PairedHostView } from "../../ipc";
import { lostLabel, reconnectingCopy } from "./model";

/** Two lines. Warning text on a 10% warning tint is 4.75:1 in light and 5.04:1 in dark.
 *  The sentence wraps on words. Only the host:port may break mid-token, or a long address overflows the card. */
export function OfflineBanner({ host, seconds }: { host: PairedHostView; seconds: number }) {
  const copy = reconnectingCopy(host.name, host.address);
  return (
    <div className="rounded-md border border-warning bg-warning/10 px-3 py-2 text-xs text-warning">
      <p>{lostLabel(seconds)}</p>
      <p className="mt-1">
        {copy.before}
        {copy.address !== null ? <span className="wrap-anywhere">{copy.address}</span> : null}
        {copy.after}
      </p>
    </div>
  );
}
