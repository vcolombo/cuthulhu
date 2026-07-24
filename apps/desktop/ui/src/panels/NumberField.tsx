// SPDX-License-Identifier: GPL-3.0-or-later
import { useRef, useState, type PointerEvent } from "react";

export function scrubValue(v: number, dx: number, step: number, min = -Infinity): number {
  return Math.max(min, v + dx * step);
}

type Props = {
  label: string;
  value: number;
  step?: number;
  min?: number;
  disabled?: boolean;
  onChange: (v: number) => void;
};

/** Numeric input whose label doubles as a drag-scrub handle (pointer dx * step).
 *  Scrubbing only previews locally as the pointer moves; onChange fires once on
 *  pointerup so one drag gesture produces one undo step. */
export function NumberField({ label, value, step = 1, min, disabled, onChange }: Props) {
  const drag = useRef<{ startX: number; startValue: number } | null>(null);
  const [preview, setPreview] = useState<number | null>(null);

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

  return (
    <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 12, color: "var(--muted)" }}>
      <span
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        style={{ cursor: disabled ? "default" : "ew-resize", userSelect: "none", width: 12 }}
      >
        {label}
      </span>
      <input
        type="number"
        value={preview ?? value}
        step={step}
        min={min}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        style={{
          width: "100%",
          background: "var(--workspace)",
          color: "var(--text)",
          border: "1px solid var(--border)",
        }}
      />
    </label>
  );
}
