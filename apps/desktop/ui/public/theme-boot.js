// SPDX-License-Identifier: GPL-3.0-or-later
// Runs before the bundle. The content security policy forbids an inline script,
// and this file cannot call into Tauri, so the only theme it can know is the last
// resolved one cached under `cuthulhu.theme` (the same key `theme.ts` writes).
// preferences.json is applied after mount, while the window is still hidden.
(function () {
  var key = "cuthulhu.theme";
  var theme = null;
  try {
    theme = localStorage.getItem(key);
  } catch (e) {
    theme = null;
  }
  if (theme !== "light" && theme !== "dark") {
    theme = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  var root = document.documentElement;
  root.classList.toggle("dark", theme === "dark");
  root.style.colorScheme = theme;
})();
