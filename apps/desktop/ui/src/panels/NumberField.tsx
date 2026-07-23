// SPDX-License-Identifier: GPL-3.0-or-later
import { useRef, type PointerEvent } from "react";

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

/** Numeric input whose label doubles as a drag-scrub handle (pointer dx * step). */
export function NumberField({ label, value, step = 1, min, disabled, onChange }: Props) {
  const drag = useRef<{ startX: number; startValue: number } | null>(null);

  const onPointerDown = (e: PointerEvent<HTMLSpanElement>) => {
    if (disabled) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { startX: e.clientX, startValue: value };
  };
  const onPointerMove = (e: PointerEvent<HTMLSpanElement>) => {
    if (!drag.current) return;
    const dx = e.clientX - drag.current.startX;
    onChange(scrubValue(drag.current.startValue, dx, step, min ?? -Infinity));
  };
  const onPointerUp = () => {
    drag.current = null;
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
        value={value}
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
