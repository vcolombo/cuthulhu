// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useId, useRef, useState, type PointerEvent } from "react";

export function scrubValue(v: number, dx: number, step: number, min = -Infinity): number {
  return Math.max(min, v + dx * step);
}

type Props = {
  label: string;
  value: number;
  step?: number;
  min?: number;
  disabled?: boolean;
  /** Shown after the field. It is the accessible description, not part of the name. */
  suffix?: string;
  onChange: (v: number) => void;
};

/** A zero, a trailing dot, or a trailing decimal zero is not finished: "0" is the start of 0.5. */
function spellingInProgress(raw: string): boolean {
  return raw === "0" || raw === "-0" || raw.endsWith(".") || (raw.includes(".") && raw.endsWith("0"));
}

/** Numeric input whose label doubles as a drag-scrub handle (pointer dx * step).
 *  Scrubbing only previews locally as the pointer moves; onChange fires once on
 *  pointerup so one drag gesture produces one undo step. */
export function NumberField({ label, value, step = 1, min, disabled, suffix, onChange }: Props) {
  const drag = useRef<{ startX: number; startValue: number } | null>(null);
  const suffixId = useId();
  const [preview, setPreview] = useState<number | null>(null);
  // An empty or half-typed value stays local. Committing Number("") is 0, which
  // would rotate or move the shape before the next digit arrives.
  const [text, setText] = useState<string | null>(null);
  useEffect(() => {
    setText((current) => {
      if (current === null || current === "") return null;
      const next = Number(current);
      // A draft that is still a different number is a keystroke the commit has not
      // landed yet. Dropping it makes the next digit edit the preview.
      if (!Number.isFinite(next) || next !== value) return current;
      return spellingInProgress(current) ? current : null;
    });
  }, [value]);

  const valueAt = (clientX: number) => {
    const d = drag.current;
    if (!d) return value;
    return scrubValue(d.startValue, clientX - d.startX, step, min ?? -Infinity);
  };

  const onPointerDown = (e: PointerEvent<HTMLSpanElement>) => {
    if (disabled) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { startX: e.clientX, startValue: value };
  };
  const onPointerMove = (e: PointerEvent<HTMLSpanElement>) => {
    if (!drag.current) return;
    setPreview(valueAt(e.clientX));
  };
  const onPointerUp = (e: PointerEvent<HTMLSpanElement>) => {
    if (!drag.current) return;
    const final = valueAt(e.clientX);
    drag.current = null;
    setPreview(null);
    if (final !== value) onChange(final);
  };
  // A canceled pointer (OS gesture, window blur, lost capture) abandons the scrub:
  // discard the preview without committing, so no stuck drag state lingers.
  const onPointerCancel = () => {
    drag.current = null;
    setPreview(null);
  };

  return (
    <div className="flex items-center gap-1 text-xs text-muted-foreground">
      <label className="flex min-w-0 flex-1 items-center gap-1">
        <span
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          onPointerCancel={onPointerCancel}
          onLostPointerCapture={onPointerCancel}
          className="w-16 shrink-0 select-none"
          style={{ cursor: disabled ? "default" : "ew-resize" }}
        >
          {label}
        </span>
        <input
          type="number"
          aria-label={label}
          aria-describedby={suffix ? suffixId : undefined}
          value={preview !== null ? preview : text !== null ? text : value}
          step={step}
          min={min}
          disabled={disabled}
          onBlur={() => setText(null)}
          onChange={(e) => {
            const raw = e.target.value;
            if (raw === "") {
              setText("");
              return;
            }
            const next = Number(raw);
            if (!Number.isFinite(next)) {
              setText(raw);
              return;
            }
            // Keep every finite keystroke. The first digit previews immediately and later
            // digits queue; clearing the draft makes the next keystroke edit that preview.
            setText(raw);
            onChange(next);
          }}
          className="w-full rounded-sm border border-input bg-background px-1 text-foreground"
        />
      </label>
      {suffix ? <span id={suffixId} className="w-5 shrink-0">{suffix}</span> : null}
    </div>
  );
}
