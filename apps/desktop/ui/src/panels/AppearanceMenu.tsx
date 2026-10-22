// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { Check, Monitor, Moon, Sun, SunMoon } from "lucide-react";
import type { Appearance } from "../theme";

type Props = {
  appearance: Appearance;
  ready: boolean;
  onAppearance: (next: Appearance) => void;
};

const OPTIONS: { value: Appearance; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "system", label: "System" },
];

function OptionIcon({ value }: { value: Appearance }) {
  const Icon = value === "light" ? Sun : value === "dark" ? Moon : Monitor;
  return <Icon className="size-3.5" aria-hidden />;
}

export function AppearanceMenu({ appearance, ready, onAppearance }: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuId = useId();

  const closeToTrigger = useCallback(() => {
    setOpen(false);
    // The focused item unmounts with the menu, and focus would land on body.
    triggerRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: PointerEvent) => {
      // The pointer target keeps focus. Only a keyboard close returns it here.
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeToTrigger();
    };
    document.addEventListener("pointerdown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, closeToTrigger]);

  const choose = (next: Appearance) => {
    onAppearance(next);
    closeToTrigger();
  };

  const onMenuKey = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const items = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'));
    const index = items.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const next = event.key === "ArrowDown" ? index + 1 : index - 1;
      items[(next + items.length) % items.length]?.focus();
    }
    if (event.key === "Home") {
      event.preventDefault();
      items[0]?.focus();
    }
    if (event.key === "End") {
      event.preventDefault();
      items[items.length - 1]?.focus();
    }
  };

  return (
    <div ref={rootRef} style={{ position: "relative" }}>
      <button
        ref={triggerRef}
        type="button"
        aria-label="Appearance"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        disabled={!ready}
        onClick={() => setOpen((value) => !value)}
        className="inline-flex size-8 cursor-pointer items-center justify-center rounded-md text-foreground hover:bg-accent disabled:cursor-default"
      >
        <SunMoon className="size-4" aria-hidden />
      </button>
      {open ? (
        <div
          id={menuId}
          role="menu"
          aria-label="Appearance"
          onKeyDown={onMenuKey}
          className="absolute right-0 z-50 mt-1 flex min-w-[11rem] flex-col gap-0.5 rounded-md border border-border bg-popover p-1 text-foreground shadow-md"
        >
          <div className="px-2 py-1.5 text-2xs font-medium uppercase tracking-wide text-muted-foreground">Appearance</div>
          {OPTIONS.map((option) => {
            const checked = appearance === option.value;
            return (
              <button
                key={option.value}
                type="button"
                role="menuitemradio"
                aria-checked={checked}
                autoFocus={checked}
                className="appearance-option flex w-full cursor-pointer items-center gap-2 rounded-sm border-0 bg-transparent px-2 py-1.5 text-left text-sm text-foreground"
                onClick={() => choose(option.value)}
              >
                <OptionIcon value={option.value} />
                <span style={{ flex: 1 }}>{option.label}</span>
                {checked ? <Check className="size-3" aria-hidden /> : <span aria-hidden style={{ width: 12 }} />}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
