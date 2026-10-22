// SPDX-License-Identifier: GPL-3.0-or-later
import { Button } from "../../components/ui/button";
import type { CutStatus, PairedHostView } from "../../ipc";
import type { VinylTheme } from "../../render/vinyl";
import { JobButtons } from "./JobButtons";
import { currentPass, loadPrompt, waitingLine, type JobSnapshot } from "./model";
import { OfflineBanner } from "./OfflineBanner";
import { VinylSwatch } from "./VinylSwatch";

type Props = {
  status: CutStatus;
  snapshot: JobSnapshot | null;
  cutter: string;
  theme: VinylTheme;
  offlineHost: PairedHostView | null;
  offlineSeconds: number;
  onCancel: () => void;
  onResume: () => void;
  onConfirm: () => void;
  onCollapse: () => void;
};

/** Full-width, above the status bar. Confirm pass done is here only when resume is also
 *  legal — otherwise the panel owns it, because that state is not a colour swap. */
export function SwapBar(props: Props) {
  const pass = currentPass(props.snapshot, props.status);
  const name = pass?.name ?? "the next vinyl";
  return (
    <section aria-label="Colour swap" className="flex flex-wrap items-center gap-4 border-t border-border bg-card px-4 py-3">
      <VinylSwatch color={pass?.swatch ?? null} theme={props.theme} className="size-14 rounded-lg" />
      <div className="min-w-0 flex-1">
        <p role="status" className="text-base font-medium text-foreground">{loadPrompt(name)}</p>
        <p className="text-xs text-muted-foreground">
          {waitingLine(props.cutter, props.status.pass, pass?.shapeCount ?? null)}
        </p>
        {props.offlineHost !== null ? (
          <div className="mt-2 max-w-xl">
            <OfflineBanner host={props.offlineHost} seconds={props.offlineSeconds} />
          </div>
        ) : null}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <JobButtons
          actions={props.status.actions}
          place="swap"
          offline={props.offlineHost !== null}
          onCancel={props.onCancel}
          onResume={props.onResume}
          onConfirm={props.onConfirm}
          extra={
            <Button variant="ghost" size="sm" onClick={props.onCollapse}>
              Collapse
            </Button>
          }
        />
      </div>
    </section>
  );
}

/** The bar, folded into the status bar. Opening it is the only action. */
export function SwapChip({
  status,
  snapshot,
  theme,
  offline,
  onExpand,
}: {
  status: CutStatus;
  snapshot: JobSnapshot | null;
  theme: VinylTheme;
  offline: boolean;
  onExpand: () => void;
}) {
  const pass = currentPass(snapshot, status);
  const name = pass?.name ?? "the next vinyl";
  const prompt = loadPrompt(name);
  return (
    <button
      type="button"
      data-testid="swap-chip"
      aria-label={`Show colour swap: ${prompt}`}
      onClick={onExpand}
      className="inline-flex max-w-64 items-center gap-2 rounded-sm border border-border bg-card px-2 text-foreground"
    >
      <VinylSwatch color={pass?.swatch ?? null} theme={theme} className="size-3 rounded-sm" />
      <span className="truncate">{offline ? "Connection lost" : prompt}</span>
    </button>
  );
}
