// SPDX-License-Identifier: GPL-3.0-or-later
import { Loader2 } from "lucide-react";
import type { PairedHostView } from "../../ipc";
import { lostBanner, lostLabel } from "./model";

/** The frozen-job banner. The address wraps in full; a truncated host is the one an operator cannot redial. */
export function OfflineBanner({ host, seconds }: { host: PairedHostView; seconds: number }) {
  return (
    <div className="rounded-md border border-warning bg-card px-3 py-2 text-xs">
      <p className="text-warning">{lostBanner(host.name)}</p>
      <p className="mt-1 flex items-center gap-1.5 text-warning">
        <Loader2 className="size-3.5 animate-spin motion-reduce:animate-none" aria-hidden />
        Reconnecting…
      </p>
      {host.address !== "" ? <p className="mt-1 break-all text-foreground">{host.address}</p> : null}
      <p className="mt-1 text-muted-foreground">Progress below is frozen at the last update we got.</p>
      <p className="text-warning">{lostLabel(seconds)}</p>
    </div>
  );
}
