// SPDX-License-Identifier: GPL-3.0-or-later
import { useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";
import { Button } from "../../components/ui/button";
import type { CutStatus, PairedHostView } from "../../ipc";
import type { VinylTheme } from "../../render/vinyl";
import { JobButtons } from "./JobButtons";
import { OfflineBanner } from "./OfflineBanner";
import {
  currentPass,
  failureCopy,
  passPercent,
  percentLabel,
  type JobSnapshot,
  type JobSurface,
} from "./model";
import { VinylSwatch } from "./VinylSwatch";

type Props = {
  status: CutStatus;
  snapshot: JobSnapshot | null;
  surface: JobSurface;
  place: { cutter: string; where: string };
  theme: VinylTheme;
  offlineHost: PairedHostView | null;
  offlineSeconds: number;
  onCancel: () => void;
  onResume: () => void;
  onConfirm: () => void;
  onStartAgain: () => void;
  onDismiss: () => void;
};

/** Docked in the right column. Pause, Retry pass (#306) and "about N min" (#309) are not
 *  rendered: the backend has neither a pause command nor a rate estimate yet. */
export function JobPanel(props: Props) {
  const [minimised, setMinimised] = useState(false);
  const { status, place, offlineHost } = props;
  const pass = currentPass(props.snapshot, status);
  const pct = passPercent(status.sent);
  const passLine = status.pass ? `Pass ${status.pass.index + 1} of ${status.pass.total}` : null;

  return (
    <section aria-label="Job" className="shrink-0 border-t border-border bg-card px-3 py-3 text-sm">
      <header className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="truncate font-medium text-foreground">{place.cutter}</div>
          {place.where !== "" ? <div className="truncate text-xs text-muted-foreground">{place.where}</div> : null}
        </div>
        <Button
          variant="ghost"
          size="icon"
          aria-label={minimised ? "Expand job" : "Minimise job"}
          onClick={() => setMinimised((value) => !value)}
        >
          {minimised ? <ChevronUp aria-hidden /> : <ChevronDown aria-hidden />}
        </Button>
      </header>
      {offlineHost !== null ? (
        <div className="mt-2">
          <OfflineBanner host={offlineHost} seconds={props.offlineSeconds} />
        </div>
      ) : null}
      {minimised ? null : (
        <div className="mt-3 flex flex-col gap-2">
          <JobBody
            {...props}
            pass={pass}
            pct={pct}
            passLine={passLine}
          />
        </div>
      )}
    </section>
  );
}

function JobBody({
  status,
  surface,
  theme,
  offlineHost,
  pass,
  pct,
  passLine,
  onCancel,
  onResume,
  onConfirm,
  onStartAgain,
  onDismiss,
}: Props & {
  pass: ReturnType<typeof currentPass>;
  pct: number | null;
  passLine: string | null;
}) {
  const offline = offlineHost !== null;
  switch (surface) {
    case "hidden":
    case "swap":
      return null;
    case "failed": {
    const copy = failureCopy(status.error);
    return (
      <>
        <h2 className="font-medium text-destructive">Cut failed</h2>
        <p>{copy.summary}</p>
        {copy.details !== "" ? (
          <details className="text-xs">
            <summary className="cursor-pointer text-muted-foreground">Details</summary>
            <p className="mt-1 break-all text-foreground">{copy.details}</p>
          </details>
        ) : null}
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={onStartAgain}>Start again</Button>
          <Button variant="ghost" size="sm" onClick={onDismiss}>Dismiss</Button>
        </div>
      </>
    );
    }
    case "completed":
    case "cancelled":
      return (
        <>
          <p role="status" className="font-medium text-foreground">{surface === "completed" ? "Completed" : "Cancelled"}</p>
          <div>
            <Button variant="ghost" size="sm" onClick={onDismiss}>Dismiss</Button>
          </div>
        </>
      );
    case "cancelling":
      return <p className="text-muted-foreground">Cancelling…</p>;
    case "progress":
    case "confirm":
      return (
    <>
      {passLine !== null ? <p className="font-medium text-foreground">{passLine}</p> : null}
      {pass !== null ? (
        <p className="flex items-center gap-2 text-foreground">
          <VinylSwatch color={pass.swatch} theme={theme} className="size-4 rounded-sm" />
          <span>{pass.name}</span>
        </p>
      ) : null}
      {pct !== null ? (
        <>
          <div
            data-testid="pass-bar"
            role="progressbar"
            aria-label="This pass"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
            className="h-2 overflow-hidden rounded-full bg-subtle"
          >
            <div
              className={offline ? "h-full bg-primary" : "progress-live h-full bg-primary"}
              style={{ width: `${pct}%` }}
            />
          </div>
          <p data-testid="pass-progress" className="text-xs text-muted-foreground">{percentLabel(pct)}</p>
        </>
      ) : null}
      <div className="flex flex-wrap gap-2">
        <JobButtons
          actions={status.actions}
          place="panel"
          offline={offline}
          onCancel={onCancel}
          onResume={onResume}
          onConfirm={onConfirm}
        />
      </div>
    </>
      );
    default: {
      const never: never = surface;
      return never;
    }
  }
}
