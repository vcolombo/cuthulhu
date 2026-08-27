// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * The webview's half of the close guard: a handshake, not a subscription.
 *
 * Tauri's `emit` returns success with nobody listening, so the native handler cannot tell an
 * installed listener from a page that has not subscribed yet. Readiness is what tells it, and the
 * ordering is the whole content of the flag: announced before a listener exists, a refused close
 * has no prompt behind it and the operator has a window that will not shut; left announced after
 * the listener is gone, the same. Both windows are real — the first at startup and after a webview
 * reload, where no effect cleanup runs, and the second at teardown.
 *
 * Extracted from the effect that used to hold it because that ordering is the invariant and an
 * effect body cannot be asserted: Playwright cannot unmount React, and this UI has no renderer
 * harness. As a function over its own dependencies it is checkable in both directions.
 */
export type CloseGuardDeps = {
  /** Tell the backend whether a listener is installed. */
  setReady: (ready: boolean) => Promise<void>;
  /** Subscribe to the backend's warning, resolving to the unsubscribe. */
  listen: (onWarning: () => void) => Promise<() => void>;
  /** What to do when the backend says a cut may be running. */
  onWarning: () => void;
  /** Where a failed handshake goes: a guard that could not be armed is not silent. */
  onError: (e: unknown) => void;
};

/**
 * Arm the guard, returning its teardown.
 *
 * Stood down first, because the backend's readiness outlives this page: a reload leaves the flag
 * set from the page before it, with nothing subscribed on this one.
 */
export function armCloseGuard({ setReady, listen, onWarning, onError }: CloseGuardDeps): () => void {
  let active = true;
  let stopListening: (() => void) | null = null;

  setReady(false)
    .then(() => listen(onWarning))
    .then((unlisten) => {
      // Torn down while this was resolving: the listener goes and readiness is never announced,
      // which is the same order the teardown below keeps.
      if (!active) {
        unlisten();
        return;
      }
      stopListening = unlisten;
      return setReady(true);
    })
    .catch(onError);

  return () => {
    active = false;
    const unlisten = stopListening;
    // The listener is kept until the backend has acknowledged `false`. Readiness must never be
    // true while the route out of a prevented close has already been removed.
    setReady(false)
      .then(() => unlisten?.())
      .catch(onError);
  };
}
