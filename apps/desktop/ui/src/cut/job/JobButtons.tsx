// SPDX-License-Identifier: GPL-3.0-or-later
import { useState, type ReactElement, type ReactNode } from "react";
import { Play } from "lucide-react";
import { Tip } from "../../components/Tip";
import { Button } from "../../components/ui/button";
import type { CutStatus } from "../../ipc";
import { CancelCutDialog } from "./CancelCutDialog";
import { cancelLabel } from "./model";

const WAITING = "Waiting for the host to reconnect";

function Held({ offline, children }: { offline: boolean; children: ReactElement }) {
  if (!offline) return children;
  return (
    <Tip label={WAITING} side="top" disabled>
      {children}
    </Tip>
  );
}

/** Cancel, confirm and resume, in that order, and only when `actions` says so.
 *  The cut action is Start cut on the Send sheet: a panel Cut would send with no plan. */
export function JobButtons({
  actions,
  place,
  offline,
  onCancel,
  onResume,
  onConfirm,
  extra,
}: {
  actions: CutStatus["actions"];
  place: "panel" | "swap";
  offline: boolean;
  onCancel: () => void;
  onResume: () => void;
  onConfirm: () => void;
  /** Sits between Cancel and the primary, so Collapse stays where the bar was drawn. */
  extra?: ReactNode;
}) {
  const [confirming, setConfirming] = useState(false);
  const confirmPrimary = actions.confirm && !actions.resume;
  return (
    <>
      {actions.cancel ? (
        <Button
          variant={place === "swap" ? "ghost" : "secondary"}
          size="sm"
          className={place === "swap" ? "text-foreground" : undefined}
          onClick={() => setConfirming(true)}
        >
          {cancelLabel(place, offline)}
        </Button>
      ) : null}
      {extra}
      {actions.confirm ? (
        <Held offline={offline}>
          <Button
            variant={confirmPrimary ? "default" : "secondary"}
            size="sm"
            disabled={offline}
            onClick={onConfirm}
          >
            Confirm pass done
          </Button>
        </Held>
      ) : null}
      {actions.resume ? (
        <Held offline={offline}>
          <Button
            variant="default"
            size={place === "swap" ? "default" : "sm"}
            className={place === "swap" ? "h-11 px-5 text-base font-semibold" : undefined}
            disabled={offline}
            onClick={onResume}
          >
            {place === "swap" ? <Play aria-hidden /> : null}
            Resume
          </Button>
        </Held>
      ) : null}
      <CancelCutDialog
        open={confirming}
        offline={offline}
        onOpenChange={setConfirming}
        onConfirm={onCancel}
      />
    </>
  );
}
