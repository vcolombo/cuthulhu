// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useRef } from "react";
import { Check, FlaskConical } from "lucide-react";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../components/ui/dropdown-menu";
import { DEMO_FIXTURES, DEMO_STORAGE_KEY, type DemoFixture, type DemoId } from "./fixtures";

type Props = {
  onImportError: (message: string) => void;
  onHostOffline: () => void;
};

function currentId(): DemoId | null {
  const raw = (window as Window & { __cuthulhuDemo?: { id?: string } }).__cuthulhuDemo;
  if (!raw || typeof raw.id !== "string") return null;
  return DEMO_FIXTURES.some((fixture) => fixture.id === raw.id) ? (raw.id as DemoId) : null;
}

/** Dev-only. The production build drops this module with the flag that imports it. */
export function DemoMenu({ onImportError, onHostOffline }: Props) {
  const applied = useRef(false);
  const current = currentId();
  const label = DEMO_FIXTURES.find((fixture) => fixture.id === current)?.label ?? null;

  useEffect(() => {
    if (applied.current || current === null) return;
    applied.current = true;
    const fixture = DEMO_FIXTURES.find((item) => item.id === current);
    if (!fixture) return;
    if (fixture.importError) onImportError(fixture.importError);
    if (fixture.hostOffline) onHostOffline();
  }, [current, onImportError, onHostOffline]);

  const choose = (fixture: DemoFixture) => {
    sessionStorage.setItem(DEMO_STORAGE_KEY, JSON.stringify(fixture));
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
