// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";
import { ipcErrorMessage, getPreferences, setPreferences } from "./ipc";
import {
  applyTheme,
  listenOsTheme,
  osTheme,
  resolveAppearance,
  showWindow,
  syncNativeTheme,
  type Appearance,
  type ResolvedTheme,
} from "./theme";

export type ThemeControls = {
  appearance: Appearance;
  resolved: ResolvedTheme;
  /** False until the first `get_preferences` attempt has settled, so a click cannot
   *  race the value that attempt is about to apply. */
  ready: boolean;
  fault: string | null;
  setAppearance: (next: Appearance) => void;
};

function bootResolved(): ResolvedTheme {
  if (typeof document === "undefined") return "light";
  return document.documentElement.classList.contains("dark") ? "dark" : "light";
}

export function useTheme(): ThemeControls {
  const [appearance, setAppearanceState] = useState<Appearance>("system");
  const [resolved, setResolved] = useState<ResolvedTheme>(bootResolved);
  const [ready, setReady] = useState(false);
  const [fault, setFault] = useState<string | null>(null);
  const appearanceRef = useRef(appearance);
  appearanceRef.current = appearance;

  const publish = useCallback((next: Appearance, os: ResolvedTheme) => {
    const resolvedTheme = resolveAppearance(next, os);
    applyTheme(resolvedTheme);
    setResolved(resolvedTheme);
  }, []);

  useEffect(() => {
    let cancelled = false;
    let stopListening = () => {};
    (async () => {
      let next: Appearance = "system";
      try {
        next = (await getPreferences()).appearance;
      } catch (e) {
        // Still show the window. A hidden window and a silent failure are the same
        // from the operator's side, and the boot script has already painted something.
        if (!cancelled) setFault(ipcErrorMessage(e));
      }
      if (cancelled) return;
      setAppearanceState(next);
      appearanceRef.current = next;
      publish(next, await osTheme());
      if (cancelled) return;
      await syncNativeTheme(next);
      stopListening = await listenOsTheme(() => {
        if (appearanceRef.current !== "system") return;
        void osTheme().then((os) => {
          if (appearanceRef.current === "system") publish("system", os);
        });
      });
      if (cancelled) {
        stopListening();
        return;
      }
      setReady(true);
      showWindow();
    })();
    return () => {
      cancelled = true;
      stopListening();
    };
  }, [publish]);

  const setAppearance = useCallback(
    (next: Appearance) => {
      void (async () => {
        try {
          const saved = await setPreferences(next);
          setFault(null);
          setAppearanceState(saved.appearance);
          appearanceRef.current = saved.appearance;
          publish(saved.appearance, await osTheme());
          await syncNativeTheme(saved.appearance);
        } catch (e) {
          setFault(ipcErrorMessage(e));
        }
      })();
    },
    [publish],
  );

  return { appearance, resolved, ready, fault, setAppearance };
}
