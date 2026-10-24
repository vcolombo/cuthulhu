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
  // Finite numbers this field sent while the draft was open, oldest first. Clearing does
  // not cancel them: transformWith may still apply the first digit and then the rest.
  // A landed value stays in the draft only when this field sent it, so "-0.5" survives
  // its 0 arriving and an undo that only looks like a prefix does not.
  const sent = useRef<number[]>([]);
  // The latest sent number, once it has arrived into an empty field. Remembering it
  // lets the list be dropped without the empty draft collapsing on a second pass.
  const caught = useRef<number | null>(null);
  const forgetSent = () => {
    sent.current = [];
    caught.current = null;
  };
  useEffect(() => {
    setText((current) => {
      if (current === null) return null;
      if (current === "") {
        // Every keystroke of the cleared number, not only the last. The first digit of
        // 12 lands before 12 does; treating it as unrelated would fill the field back in.
        if (caught.current === value) return "";
        const last = sent.current[sent.current.length - 1];
        if (last !== undefined && sent.current.some((n) => n === value)) {
          if (value === last) {
            caught.current = value;
            sent.current = [];
          }
          return "";
        }
        forgetSent();
        return null;
      }
      const next = Number(current);
      const last = sent.current[sent.current.length - 1];
      if (!Number.isFinite(next)) {
        forgetSent();
        return null;
      }
      if (next === value) {
        if (!spellingInProgress(current)) forgetSent();
        return spellingInProgress(current) ? current : null;
      }
      // An earlier keystroke from this draft has landed; the latest is still in flight.
      if (last === next && sent.current.some((n) => n === value)) return current;
      forgetSent();
      return null;
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
    // The drag is a new value. Leaving the typed draft up would hide where the shape moved.
    forgetSent();
    setText(null);
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
          onBlur={() => {
            // Blur abandons the draft, so a late acknowledgment of what was typed should show.
            forgetSent();
            setText(null);
          }}
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
            sent.current.push(next);
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
