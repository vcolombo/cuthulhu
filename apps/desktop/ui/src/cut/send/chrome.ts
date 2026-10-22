// SPDX-License-Identifier: GPL-3.0-or-later
import type { CSSProperties } from "react";
import { deviceBadge } from "../../hosts/deviceList";

export const panelStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 100,
};

export const dialogStyle: CSSProperties = {
  background: "var(--panel)",
  border: "1px solid var(--border)",
  color: "var(--text)",
  padding: 16,
  width: 640,
  maxHeight: "85vh",
  overflow: "auto",
  display: "flex",
  flexDirection: "column",
  gap: 10,
};

// `deviceBadge` decides the tone; the palette is this file's, since a pure module has no
// business knowing the tokens. Without this the label rendered in one flat grey, so "Cutting",
// "Ready" and "Unreachable" were the same small muted text as the machine id beside them —
// six cutters across two hosts with one mid-cut, and nothing on screen picking it out.
// Red belongs to `attention` alone, which is the tone that asks for a person. `unknown` is the
// state of every cutter on a freshly opened dialog, so it keeps the muted weight the row had
// before any of this: quiet is what "nobody has asked yet" should look like.
export const TONE_COLOR: Record<ReturnType<typeof deviceBadge>["tone"], string> = {
  idle: "var(--ready)",
  busy: "var(--accent-ink)",
  attention: "var(--cut)",
  unknown: "var(--muted-foreground)",
  gone: "var(--muted-foreground)",
};

export const btn: CSSProperties = {
  background: "var(--panel)",
  color: "var(--text)",
  border: "1px solid var(--border)",
  padding: "4px 10px",
  cursor: "pointer",
};
