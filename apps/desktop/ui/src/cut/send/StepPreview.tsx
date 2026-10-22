// SPDX-License-Identifier: GPL-3.0-or-later
import type { ReactNode } from "react";
import type { Scene } from "../../render/hittest";
import type { ResolvedTheme } from "../../theme";
import { CutPreview } from "../CutPreview";
import type { PassRow } from "./useCutPlan";

/** The host had already accepted this dispatch. It sits above the grouping control, where the
 *  single scroll has always put it; the preview itself is the mat below the passes. */
export function AlreadyAcceptedNote({ show }: { show: boolean }) {
  if (!show) return null;
  return (
    <div role="alert" style={{ color: "var(--cut)", fontSize: 12 }}>
      This Cut Host had already accepted this cut, so nothing new was started — this was read
      as a retry of a dispatch whose answer was lost. If the cutter is idle and you meant a
      fresh sheet, press Cut again.
    </div>
  );
}

export function StepPreview({
  scene,
  artboard,
  passes,
  travel,
  theme,
}: {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  passes: PassRow[];
  travel: [number, number, number, number][];
  theme: ResolvedTheme;
}): ReactNode {
  return <CutPreview scene={scene} artboard={artboard} passes={passes} travel={travel} theme={theme} />;
}
