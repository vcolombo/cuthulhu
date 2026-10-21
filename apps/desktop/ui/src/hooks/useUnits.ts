// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useState } from "react";
import type { LengthUnit } from "../length";
import * as ipc from "../ipc";

/** Lengths on screen. Stored documents stay in millimetres; millimetres is the unset choice. */
export function useUnits(onError: (message: string) => void) {
  const [unit, setUnitState] = useState<LengthUnit>("mm");

  useEffect(() => {
    ipc
      .getPreferences()
      .then((prefs) => setUnitState(prefs.units))
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
  }, [onError]);

  const setUnit = useCallback(
    (next: LengthUnit) => {
      // Appearance is omitted so a unit change does not rewrite the theme a moment later.
      ipc
        .setPreferences(undefined, next)
        .then((prefs) => setUnitState(prefs.units))
        .catch((e) => onError(ipc.ipcErrorMessage(e)));
    },
    [onError],
  );

  return { unit, setUnit };
}
