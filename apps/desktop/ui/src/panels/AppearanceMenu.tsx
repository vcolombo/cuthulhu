// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useId, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
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

/** Sun over a moon, the control the menu hangs off. Decorative: the button's name is Appearance. */
function SunMoonIcon() {
  return (
    <svg width={16} height={16} viewBox="0 0 16 16" aria-hidden fill="none" stroke="currentColor" strokeWidth={1.25}>
      <circle cx={7} cy={7} r={2.25} />
      <path d="M7 2.2v1.3M7 10.5v1.3M2.2 7h1.3M10.5 7h1.3M3.6 3.6l.9.9M9.5 9.5l.9.9M3.6 10.4l.9-.9M9.5 4.5l.9-.9" />
      <path d="M10.2 10.2a3.2 3.2 0 1 1-3.4-4.6 2.6 2.6 0 0 0 3.4 4.6z" fill="currentColor" stroke="none" />
    </svg>
  );
}

function OptionIcon({ value }: { value: Appearance }) {
  if (value === "light") {
    return (
      <svg width={14} height={14} viewBox="0 0 16 16" aria-hidden fill="none" stroke="currentColor" strokeWidth={1.25}>
        <circle cx={8} cy={8} r={2.4} />
        <path d="M8 1.8v1.6M8 12.6v1.6M1.8 8h1.6M12.6 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1" />
      </svg>
    );
  }
  if (value === "dark") {
    return (
      <svg width={14} height={14} viewBox="0 0 16 16" aria-hidden fill="currentColor">
        <path d="M9.2 2.1a5.2 5.2 0 1 0 4.7 7.6A4.4 4.4 0 0 1 9.2 2.1z" />
      </svg>
    );
  }
  return (
    <svg width={14} height={14} viewBox="0 0 16 16" aria-hidden fill="none" stroke="currentColor" strokeWidth={1.25}>
      <rect x={2} y={3} width={12} height={8} rx={1} />
      <path d="M6 13.5h4M8 11v2.5" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg width={12} height={12} viewBox="0 0 12 12" aria-hidden fill="none" stroke="currentColor" strokeWidth={1.5}>
      <path d="M2 6.2 4.6 9 10 3" />
    </svg>
  );
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
        className="inline-flex size-8 cursor-pointer items-center justify-center rounded-md text-foreground hover:bg-accent disabled:cursor-default disabled:opacity-[0.35]"
      >
        <SunMoonIcon />
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
                {checked ? <CheckIcon /> : <span aria-hidden style={{ width: 12 }} />}
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}
