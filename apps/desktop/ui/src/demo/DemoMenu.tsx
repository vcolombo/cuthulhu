// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useRef } from "react";
import { Check, FlaskConical } from "lucide-react";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import type { JobSnapshot } from "../cut/job/model";
import { DEMO_FIXTURES, DEMO_STORAGE_KEY, IMPORT_FAILED, type DemoFixture, type DemoId } from "./fixtures";

type Props = {
  onError: (message: string) => void;
  remember: (snapshot: JobSnapshot) => void;
};

// jobId 0 is the accept that carries no id: a non-zero id already on screen would skip the snapshot.
// The host-offline card mounts only once a job is remembered, and this module is what the production build drops.
const HOST_OFFLINE_JOB: JobSnapshot = {
  grouping: "Single",
  passes: [{ key: "all", shapeCount: 2, enabled: true, name: "All shapes", swatch: null }],
  jobId: 0,
};

function currentId(): DemoId | null {
  const raw = (window as Window & { __cuthulhuDemo?: { id?: string } }).__cuthulhuDemo;
  if (!raw || typeof raw.id !== "string") return null;
  return DEMO_FIXTURES.some((fixture) => fixture.id === raw.id) ? (raw.id as DemoId) : null;
}

/** Dev-only. The production build drops this module with the flag that imports it. */
export function DemoMenu({ onError, remember }: Props) {
  const applied = useRef(false);
  const current = currentId();
  const label = DEMO_FIXTURES.find((fixture) => fixture.id === current)?.label ?? null;

  useEffect(() => {
    if (applied.current || current === null) return;
    applied.current = true;
    switch (current) {
      case "importFailed":
        onError(IMPORT_FAILED);
        break;
      case "hostOffline":
        remember(HOST_OFFLINE_JOB);
        break;
      case "normal":
      case "noCutters":
      case "emptyCanvas":
        break;
      default: {
        const never: never = current;
        throw new Error(`unhandled demo state: ${never}`);
      }
    }
  }, [current, onError, remember]);

  const choose = (fixture: DemoFixture) => {
    try {
      sessionStorage.setItem(DEMO_STORAGE_KEY, JSON.stringify({ id: fixture.id }));
    } catch {
      onError("Couldn't save the demo state.");
      return;
    }
    window.location.reload();
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={label ? `Demo state: ${label}` : "Demo state"}
          className="flex h-6 shrink-0 items-center gap-1 rounded-sm border border-dashed border-border bg-panel px-1.5 text-2xs text-muted-foreground"
        >
          <FlaskConical className="size-3.5" aria-hidden />
          Demo
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="start" className="w-52">
        {DEMO_FIXTURES.map((fixture) => (
          <DropdownMenuItem key={fixture.id} onSelect={() => choose(fixture)}>
            <span className="min-w-0 flex-1 truncate">{fixture.label}</span>
            {fixture.id === current ? <Check className="size-3.5" aria-hidden /> : null}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
