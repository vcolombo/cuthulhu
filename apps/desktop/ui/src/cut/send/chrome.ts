// SPDX-License-Identifier: GPL-3.0-or-later
import { deviceBadge } from "../../hosts/deviceList";

// `deviceBadge` decides the tone. The classes are tokens, not a second palette: a flat
// grey label made "Cutting", "Ready" and "Unreachable" the same small text. Red belongs
// to `attention` alone, which is the tone that asks for a person. `unknown` is every
// cutter on a freshly opened dialog, so it stays quiet.
export const TONE_BADGE: Record<ReturnType<typeof deviceBadge>["tone"], string> = {
  idle: "border-success/40 bg-success/15 text-success",
  busy: "border-accent-ink/40 bg-tint/25 text-accent-ink",
  attention: "border-destructive/40 bg-destructive/15 text-destructive",
  unknown: "border-border bg-muted text-muted-foreground",
  gone: "border-border bg-muted text-muted-foreground",
};
