// SPDX-License-Identifier: GPL-3.0-or-later
import type { JobSnapshot } from "../cut/job/model";
import { DemoMenu } from "./DemoMenu";

type Props = {
  onError: (message: string) => void;
  remember: (snapshot: JobSnapshot) => void;
};

/**
 * The host-offline card is the job panel's own banner. It only mounts once a job is remembered,
 * and remembering from here keeps that call in the module the production build drops.
 * jobId 0 is the accept that carries no id: a non-zero id already on screen would skip the snapshot.
 */
export function DemoSlot({ onError, remember }: Props) {
  return (
    <DemoMenu
      onImportError={onError}
      onHostOffline={() =>
        remember({
          grouping: "Single",
          passes: [{ key: "all", shapeCount: 2, enabled: true, name: "All shapes", swatch: null }],
          jobId: 0,
        })
      }
    />
  );
}
