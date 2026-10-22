// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";
import { unitFromPreference, type LengthUnit } from "../length";
import * as ipc from "../ipc";

/** Lengths on screen. Stored documents stay in millimetres. */
export function useUnits(onError: (message: string) => void) {
  const [unit, setUnitState] = useState<LengthUnit>("mm");
  // Set in the click, before the first-run save returns. That save already read a null unit,
  // so without this it would write the region over the choice the operator just made.
  const explicit = useRef<LengthUnit | null>(null);

  useEffect(() => {
    let cancelled = false;
    ipc
      .getPreferences()
      .then(async (prefs) => {
        if (cancelled || explicit.current !== null) return;
        // No saved unit: resolve the computer's region once and write it back.
        // A later region change must not move a choice that is already in the file.
        const choice = unitFromPreference(prefs.units, navigator.language);
        if (cancelled || explicit.current !== null) return;
        setUnitState(choice.unit);
        if (!choice.save) return;
        const saved = await ipc.setPreferences(undefined, choice.unit);
        if (cancelled) return;
        const chosen = explicit.current;
        if (chosen !== null) {
          const again = await ipc.setPreferences(undefined, chosen);
          if (!cancelled && (again.units === "in" || again.units === "mm")) setUnitState(again.units);
          return;
        }
        if (saved.units === "in" || saved.units === "mm") setUnitState(saved.units);
      })
      .catch((e) => {
        if (!cancelled) onError(ipc.ipcErrorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [onError]);

  const setUnit = useCallback(
    (next: LengthUnit) => {
      explicit.current = next;
      setUnitState(next);
      // Appearance is omitted so a unit change does not rewrite the theme a moment later.
      ipc
        .setPreferences(undefined, next)
        .then((prefs) => {
          if (explicit.current !== next) return;
          if (prefs.units === "in" || prefs.units === "mm") setUnitState(prefs.units);
        })
        .catch((e) => onError(ipc.ipcErrorMessage(e)));
    },
    [onError],
  );

  return { unit, setUnit };
}
