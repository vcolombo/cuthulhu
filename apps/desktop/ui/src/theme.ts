// SPDX-License-Identifier: GPL-3.0-or-later
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Last resolved theme, not the preference. The boot script reads the same key and
 *  cannot call `get_preferences`, which is why this cache exists at all. */
export const THEME_BOOT_KEY = "cuthulhu.theme";

export type Appearance = "light" | "dark" | "system";
export type ResolvedTheme = "light" | "dark";

export function resolveAppearance(appearance: Appearance, os: ResolvedTheme): ResolvedTheme {
  return appearance === "system" ? os : appearance;
}

export function mediaTheme(): ResolvedTheme {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** The e2e fake installs `invoke` and not `metadata`. Calling the window plugin
 *  from there is an unregistered command, and it would fail every test. */
function tauriWindowAvailable(): boolean {
  const internals = (globalThis as { __TAURI_INTERNALS__?: { metadata?: unknown } }).__TAURI_INTERNALS__;
  return internals?.metadata != null;
}

export function applyTheme(resolved: ResolvedTheme): void {
  const root = document.documentElement;
  root.classList.toggle("dark", resolved === "dark");
  root.style.colorScheme = resolved;
  try {
    localStorage.setItem(THEME_BOOT_KEY, resolved);
  } catch {
    // A store that throws must not undo the class. The next launch uses matchMedia.
  }
}

/** The page theme while Appearance is System. Tauri's window theme is what native
 *  chrome follows; WebKitGTK does not always update it, so a disagreement is
 *  resolved towards `matchMedia`, which does follow the OS. */
export async function osTheme(): Promise<ResolvedTheme> {
  const media = mediaTheme();
  if (!tauriWindowAvailable()) return media;
  try {
    const reported = await getCurrentWindow().theme();
    if (reported === media) return reported;
  } catch {
    // Fall through to matchMedia.
  }
  return media;
}

export async function syncNativeTheme(appearance: Appearance): Promise<void> {
  if (!tauriWindowAvailable()) return;
  try {
    await getCurrentWindow().setTheme(appearance === "system" ? null : appearance);
  } catch {
    // Native chrome lagging the page is visible. Hiding the window over it is not.
  }
}

/** After the frame that painted `applyTheme`. One animation frame runs before that
 *  paint, so the show is the frame after it — a single callback would reveal the
 *  webview still on the previous theme. */
export function showWindow(): void {
  if (!tauriWindowAvailable()) return;
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      void getCurrentWindow().show();
    });
  });
}

export async function listenOsTheme(onChange: () => void): Promise<() => void> {
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  const onMedia = () => onChange();
  media.addEventListener("change", onMedia);
  let unlistenWindow = () => {};
  if (tauriWindowAvailable()) {
    try {
      unlistenWindow = await getCurrentWindow().onThemeChanged(() => onChange());
    } catch {
      unlistenWindow = () => {};
    }
  }
  return () => {
    media.removeEventListener("change", onMedia);
    unlistenWindow();
  };
}
