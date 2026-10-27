// SPDX-License-Identifier: GPL-3.0-or-later
import { test, expect, type Locator, type Page } from "@playwright/test";
import { modeRgb, pillApart, renderedContrastFromPixels, type Rgb } from "./renderedContrast";
// Generated from the desktop's own generate_handler! registry by
// apps/desktop/tests/ipc_inventory.rs. Nothing else here calls a real #[tauri::command], so this
// is what the fake in mockBackend.ts checks command and argument names against (#85).
// The attribute is not decoration: Playwright loads this file as ESM, where a JSON import without
// one is a TypeError before any test is collected.
import ipcInventory from "../../ipc-inventory.json" with { type: "json" };
import { installMockTauri } from "./mockBackend";

/** The right column shows one panel. Open the tab a locator lives in, and leave it when it is already open. */
async function showTab(page: Page, name: "Layers" | "Properties" | "Devices") {
  const tab = page.getByRole("tab", { name, exact: true });
  if ((await tab.getAttribute("data-state")) !== "active") await tab.click();
}

/** Save, Open and Revert live in the File menu. Clicking the menu is a pointer event; the drag test does not use this. */
async function fileAction(page: Page, name: string) {
  await page.getByRole("button", { name: "File", exact: true }).click();
  await page.getByRole("menuitem", { name, exact: true }).click();
}

const SEND_STEPS = ["Machine", "Material & passes", "Preview & cut"] as const;

/** Blank speed and force on a pass that names no preset. An unread or unknown preset is left
 *  alone: the backend resolves it, and filling it would hide the case a test is proving. */
async function fillSendLevels(page: Page) {
  const dialog = page.getByRole("dialog", { name: "Send to cutter" });
  const speeds = dialog.getByLabel(/Speed for pass /);
  const n = await speeds.count();
  for (let i = 0; i < n; i++) {
    const speed = speeds.nth(i);
    if (!(await speed.isVisible()) || !(await speed.isEnabled())) continue;
    if ((await speed.inputValue()) !== "") continue;
    const preset = dialog.getByLabel(`Preset for pass ${i + 1}`);
    if ((await preset.inputValue()) !== "no-preset") continue;
    await speed.fill("10");
    const force = dialog.getByLabel(`Force for pass ${i + 1}`);
    if ((await force.isEnabled()) && (await force.inputValue()) === "") await force.fill("10");
  }
}

/** Step 1 is the machine list, step 2 the passes, step 3 the preview and Start cut.
 *  Leaving step 2 fills the blanks a no-preset pass would otherwise refuse. A step already
 *  opened stays on the stepper when Next is gated, which is how a dropped aim is still readable. */
async function goToSendStep(page: Page, step: 1 | 2 | 3) {
  const dialog = page.getByRole("dialog", { name: "Send to cutter" });
  for (let i = 0; i < 4; i++) {
    const current = Number(await dialog.getAttribute("data-send-step"));
    if (current === step) return;
    if (current === 2 && step > current) await fillSendLevels(page);
    if (current < step) {
      const next = dialog.getByRole("button", { name: "Next", exact: true });
      if (await next.isEnabled()) {
        await next.click();
        continue;
      }
      const via = dialog.getByRole("button", { name: SEND_STEPS[step - 1] });
      if (await via.isEnabled()) {
        await via.click();
        continue;
      }
      await next.click();
    } else {
      await dialog.getByRole("button", { name: "Back", exact: true }).click();
    }
  }
  throw new Error(`send sheet did not reach step ${step}`);
}

/** The card is the connect action. The local Cameo is the first radio of that model. */
async function chooseCutter(page: Page, model: "Silhouette Cameo 5 Alpha" | "GCC Puma IV") {
  await page.getByRole("dialog", { name: "Send to cutter" }).getByRole("radio", { name: model, exact: true }).first().click();
}

/** Pass rows, grouping and weeding live on step 2, which needs a cutter that can cut. */
async function showPasses(page: Page) {
  const dialog = page.getByRole("dialog", { name: "Send to cutter" });
  await expect(dialog).toBeVisible();
  if (Number(await dialog.getAttribute("data-send-step")) !== 2) {
    const next = dialog.getByRole("button", { name: "Next", exact: true });
    if (await next.isDisabled()) await chooseCutter(page, "Silhouette Cameo 5 Alpha");
    await goToSendStep(page, 2);
  }
}

/** Weeding starts closed unless a border is already on, and the controls are not in the page until then. */
async function openWeeding(page: Page) {
  const toggle = page.getByRole("button", { name: "Weeding", exact: true });
  if ((await toggle.getAttribute("aria-expanded")) !== "true") await toggle.click();
}

/** The grip replaces Up and Down. Alt+Arrow is the keyboard half of the same move. */
async function reorderPass(page: Page, index: number, dir: -1 | 1) {
  await page.getByRole("button", { name: `Reorder pass ${index + 1}` }).press(dir < 0 ? "Alt+ArrowUp" : "Alt+ArrowDown");
}

/** Start cut exists only on step 3. Calling this from an earlier step is the walk there.
 *  A closed sheet is the job panel's doing: Start cut closes it, and a later cut opens a fresh one. */
async function startCut(page: Page) {
  if ((await page.getByRole("dialog", { name: "Send to cutter" }).count()) === 0) {
    await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  }
  await goToSendStep(page, 3);
  await page.getByRole("button", { name: "Start Cut" }).click();
}

/** The preview is step 3. Next waits out a replan, so the name is the plan that landed. */
async function showPreview(page: Page) {
  await goToSendStep(page, 3);
  return page.getByRole("img", { name: /Cut preview/ });
}

/** Cutting stays available: step 3's Start cut is enabled, then the passes are on screen again.
 *  The sheet closes when a cut starts, so a check after a job opens it again. */
async function expectStartOffered(page: Page) {
  if ((await page.getByRole("dialog", { name: "Send to cutter" }).count()) === 0) {
    await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  }
  const dialog = page.getByRole("dialog", { name: "Send to cutter" });
  const back = Number(await dialog.getAttribute("data-send-step")) < 3;
  await goToSendStep(page, 3);
  await expect(dialog.getByRole("button", { name: "Start Cut" })).toBeEnabled();
  if (back) await goToSendStep(page, 2);
}

/** The preset editor is closed until asked, and a reopened sheet starts on the machine step. */
async function showPresetEditor(page: Page) {
  await showPasses(page);
  const manage = page.getByLabel("Preset to manage");
  if (!(await manage.isVisible())) await page.getByRole("button", { name: "Edit presets…" }).click();
  await expect(manage).toBeVisible();
}

// The inventory travels in its own init script because `addInitScript` passes one argument and
// `opts` already has it, and because the fake is serialized into the page and so can close over
// nothing outside itself — including this import.
test.beforeEach(async ({ page }) => {
  await page.addInitScript((inventory) => {
    const injected = window as unknown as { __IPC_INVENTORY__: unknown };
    // Prototype-less: a lookup table, so `Object.prototype`'s members are not commands.
    injected.__IPC_INVENTORY__ = Object.assign(Object.create(null), inventory);
  }, ipcInventory as Record<string, string[]>);
});

// Every test's own check on the seam: a call the registered commands do not declare fails the test
// that made it, by name, whether or not the frontend showed anything for it.
test.afterEach(async ({ page }) => {
  // A test that failed before its first navigation invoked nothing, and about:blank has no storage
  // of its own to read.
  if (!page.url().startsWith("http")) return;
  const refused = await page.evaluate(() => {
    const stored: unknown = JSON.parse(sessionStorage.getItem("__ipc_violations__") ?? "[]");
    return Array.isArray(stored) ? stored : [];
  });
  expect(refused, "the fake refused an IPC call the desktop's registered commands do not declare").toEqual([]);
});

/** The gate every other test in this file now leans on. Both refusals are stated here because a
 *  gate that stopped biting would otherwise show up as nothing at all: every test would keep
 *  passing, which is the failure #85 is about. */
test("a command or key the registered commands do not declare is refused", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");

  const call = (cmd: string, args: Record<string, unknown>) =>
    page.evaluate(
      (c) => {
        // The fake's own channel, as everywhere else in this file.
        const internals = window as unknown as {
          __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
        };
        return internals.__TAURI_INTERNALS__.invoke(c.cmd, c.args).then(
          () => "resolved",
          (e: unknown) => (e instanceof Error ? e.message : String(e)),
        );
      },
      { cmd, args },
    );

  expect(await call("trace_imagee", {})).toBe("unregistered command: trace_imagee");
  // `Object.prototype`'s members are not commands: a lookup that found one would read a function
  // as a declared argument list and throw on it instead of refusing the call.
  expect(await call("toString", {})).toBe("unregistered command: toString");

  // The rename that shipped green for two commits: Rust's parameter is `controls`, and `ipc.ts`
  // kept sending `opts` (#85). A registered command is not a licence to send it anything.
  expect(await call("trace_image", { path: "/tmp/fake.png", opts: {} })).toBe(
    "undeclared argument for trace_image: opts",
  );

  // The same call as the frontend makes it, which the gate must let through — a check that refuses
  // everything is indistinguishable from one that refuses nothing.
  expect(await call("trace_image", { path: "/tmp/fake.png", controls: {} })).toBe("resolved");

  // Cleared because they were the point: the afterEach above fails any test that leaves one.
  await page.evaluate(() => sessionStorage.removeItem("__ipc_violations__"));
});

test("the demo state menu is absent unless the dev flag is set", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Rectangle", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /Demo state/ })).toHaveCount(0);
});

test("new doc → add rect → save → reload keeps the rect", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Rectangle", exact: true }).click();
  await page.mouse.click(400, 300);
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
  await fileAction(page, "Save");

  // Discriminating step: delete the rect after Save so Reload can only pass by genuinely
  // restoring the saved copy, not by leaving live state untouched (a no-op load_project
  // would otherwise pass the final assertion below for free).
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").click();
  await page.getByRole("button", { name: "Delete" }).click();
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(0);

  await fileAction(page, "Revert to saved");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
});

test("two-color doc cuts through swap and resume", async ({ page }) => {
  // Two differently-stroked rects are seeded synchronously inside the mock (no stroke
  // picker exists in the UI) so App.tsx's initial snapshot() already sees them.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await startCut(page);
  // Start cut closes the sheet. The job stays, including across a reopen (C2).
  await expect(page.getByRole("dialog")).toHaveCount(0);
  const swap = page.getByRole("region", { name: "Colour swap" });
  await expect(swap).toBeVisible();
  await expect(swap.getByText("Load #00ff00, then press Resume")).toBeVisible();
  // §5.2 keeps Send as the one primary. The status-bar chip is still there while the bar is open.
  await expect(page.getByRole("button", { name: "Send to cutter", exact: true })).toBeVisible();
  await expect(page.getByTestId("job-chip")).toBeVisible();

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Send to cutter" })).not.toContainText("then press Resume");
  await page.getByRole("button", { name: "Close" }).click();
  await expect(swap).toBeVisible();

  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByRole("region", { name: "Job" }).getByText("Completed", { exact: true })).toBeVisible();
  await expect(page.getByRole("dialog")).toHaveCount(0);
});

test("a disabled middle pass names the next vinyl the machine will cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedThreeVinyl: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(3);
  await page.getByRole("checkbox", { name: "Enabled" }).nth(1).uncheck();
  await page.evaluate(() => (window as unknown as { __holdPass: () => void }).__holdPass());
  await startCut(page);

  const job = page.getByRole("region", { name: "Job" });
  await expect(job.getByText("Pass 1 of 2")).toBeVisible();
  await expect(job.getByText("Red vinyl")).toBeVisible();
  await page.evaluate(() => (window as unknown as { __releasePass: () => void }).__releasePass());
  const swap = page.getByRole("region", { name: "Colour swap" });
  await expect(swap.getByText("Load gold vinyl, then press Resume")).toBeVisible();
  await expect(swap.getByText("#00ff00")).toHaveCount(0);
  await expect(swap.getByText(/pass 2 of 2 · 1 shape/)).toBeVisible();
});

// The operator-facing half of #148: the picker, the replan it triggers, and a row named for
// what it holds rather than for a colour it does not have. Nothing else in this suite selects a
// grouping, so a picker wired to a mode the backend ignores would leave every other test green.
test("changing the grouping replans and renames the passes", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByLabel("Group passes by").selectOption("Single");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  await expect(page.getByTestId("cut-pass-row").getByText("Every cut shape")).toBeVisible();

  // And back again: switching modes replans each time rather than keeping the first answer.
  await page.getByLabel("Group passes by").selectOption("Stroke");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
});

// The race the dialog's installed-plan state exists to prevent: while a replan is parked, the
// rows on screen still belong to the previous grouping, and sending them under the new one
// would cut whatever that mode happens to key the same way. Cut has to be unavailable until
// the new plan lands — a fact only a held reply can state.
test("a cut cannot be sent with rows from the previous grouping", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await fillSendLevels(page);
  const next = page.getByRole("button", { name: "Next", exact: true });
  await expect(next).toBeEnabled();
  await expect(page.getByRole("button", { name: "Start Cut" })).toHaveCount(0);

  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByLabel("Group passes by").selectOption("Single");
  await expect(next).toBeDisabled();
  // Still showing the old mode's rows, which is exactly why Cut is unavailable.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.evaluate(() => (window as unknown as { __releasePlans: () => void }).__releasePlans());
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  // The new plan's pass is a new row, so its speed and force start blank and have to be set.
  await fillSendLevels(page);
  await expect(next).toBeEnabled();
  await expectStartOffered(page);

  // The cut that then goes out names the grouping on screen, not the colour passes it replaced.
  await startCut(page);
  const request = await callFake(page, "__test_last_cut_request") as { grouping: string; passes: { key: string }[] };
  expect(request.grouping).toBe("Single");
  expect(request.passes.map((pass) => pass.key)).toEqual(["all"]);
});

test("a weed border and lines replan into the preview, and the cut carries the options", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  // Lines wait for a border: without one their ends would stop in the open sheet.
  await openWeeding(page);
  await expect(page.getByLabel("Weed lines")).toBeDisabled();
  await expect(page.getByLabel("Weed margin")).toBeDisabled();
  let preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");
  await goToSendStep(page, 2);

  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  // One border per pass, each its own sheet, and the blade travels to each after its shapes.
  await expect(page.getByLabel("Weed lines")).toBeEnabled();
  await expect(page.getByLabel("Weed spacing")).toBeDisabled();
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
  await goToSendStep(page, 2);

  await page.getByLabel("Weed lines").selectOption("Horizontal");
  await page.getByLabel("Weed spacing").fill("5");
  // Each 10 mm rect's border runs -3..13 on y; lines at y = 2 and 7 cross the rect and are cut off
  // within the clearance, leaving stubs under 2 mm, so only y = 12 survives: one line per pass.
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 5 travel moves, 4 weed paths");

  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
  const request = await callFake(page, "__test_last_cut_request") as { weed: unknown };
  expect(request.weed).toEqual({ margin_mm: 3, lines: "Horizontal", spacing_mm: 5, clearance_mm: 1.5 });
});

test("a weed field out of range disables Cut and says why", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await fillSendLevels(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  const next = page.getByRole("button", { name: "Next", exact: true });
  await expect(next).toBeEnabled();
  await expect(page.getByRole("button", { name: "Start Cut" })).toHaveCount(0);

  await page.getByLabel("Weed margin").fill("60");
  await expect(page.getByRole("alert").filter({ hasText: "Margin must be 0.5–50 mm" })).toBeVisible();
  await expect(next).toBeDisabled();
  // Not cut without its border: the last plan still holds the old margin, so Cut waits.
  await page.getByLabel("Weed margin").fill("4");
  await expect(next).toBeEnabled();

  await page.getByLabel("Weed lines").selectOption("Both");
  await page.getByLabel("Weed clearance").fill("4");
  await expect(page.getByRole("alert").filter({ hasText: "Clearance must be less than the margin" })).toBeVisible();
  await expect(next).toBeDisabled();
});

// What a replan for a weed edit does to the rows: the operator's arrangement (which passes are cut,
// in what order, with what settings) stays, since the weed changes none of the passes.
test("a weed edit keeps the passes as the operator arranged them, and their travel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  const rows = page.getByTestId("cut-pass-row");
  await expect(rows).toHaveCount(2);
  await reorderPass(page, 1, -1); // green first
  await rows.nth(1).getByRole("checkbox").uncheck(); // and red not cut
  let preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 0 travel moves");
  await goToSendStep(page, 2);

  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  await expect(rows.nth(0).locator("span").first()).toHaveCSS("background-color", "rgb(0, 255, 0)");
  await expect(rows.nth(1).getByRole("checkbox")).not.toBeChecked();
  // Travel for the list as arranged: only green is cut, so one move, to its border. The plan's
  // own travel (both passes, in planned order) would read 3.
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 1 travel move, 1 weed path");
});

// Reordering asks the backend for travel again, and that request must carry the weed the plan was
// made with: without it the replanned travel drops the moves to each border.
test("reordering passes keeps the weed in the replanned travel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  let preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
  await goToSendStep(page, 2);
  await reorderPass(page, 1, -1);
  await expect(page.getByTestId("cut-pass-row").first().locator("span").first()).toHaveCSS("background-color", "rgb(0, 255, 0)");
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
});

test("Cut waits while a weed replan is out, so a border nobody has previewed cannot be cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await fillSendLevels(page);
  const next = page.getByRole("button", { name: "Next", exact: true });
  await expect(next).toBeEnabled();
  await expect(page.getByRole("button", { name: "Start Cut" })).toHaveCount(0);
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  await expect(next).toBeDisabled();
  await page.evaluate(() => (window as unknown as { __releasePlans: () => Promise<void> }).__releasePlans());
  await expect(next).toBeEnabled();
  await expectStartOffered(page);
});

test("a refused weed replan puts the controls back to the plan still installed", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  let preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
  await goToSendStep(page, 2);

  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByLabel("Weed lines").selectOption("Both");
  // The controls say what the installed plan holds, so Cut is offered for what the preview shows.
  await expect(page.getByLabel("Weed lines")).toHaveValue("None");
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
  await expectStartOffered(page);
});

// A refusal arriving after the operator has typed on must not put back over what they typed.
test("a refused weed replan does not overwrite what was typed after it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  const preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName(/2 weed paths/);
  await goToSendStep(page, 2);
  await page.evaluate(() => {
    const w = window as unknown as { __armHold: () => void; __refuseHeldPlans: () => void };
    w.__armHold();
    w.__refuseHeldPlans();
  });
  await page.getByLabel("Weed margin").fill("5"); // replan out, held
  await page.getByLabel("Weed margin").fill(""); // mid-edit: reads as nothing, plans nothing
  await page.evaluate(() => (window as unknown as { __releasePlans: () => Promise<void> }).__releasePlans());
  await expect(page.getByLabel("Weed margin")).toHaveValue("");
});

test("changing the grouping keeps the weed", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  await page.getByLabel("Group passes by").selectOption("Single");
  // One pass over both rects, so one border around both.
  const preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 1 travel move, 1 weed path");
});

test("Border waits for the weed defaults, and says why while it waits or when they fail", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.evaluate(() => (window as unknown as { __holdRanges: () => void }).__holdRanges());
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await openWeeding(page);
  const border = page.getByLabel("Weed border");
  // Ticked now it would start from an empty margin.
  await expect(border).toBeDisabled();
  await expect(border).toHaveAttribute("title", "Waiting for the weed defaults");
  await page.evaluate(() => (window as unknown as { __releaseRanges: () => Promise<void> }).__releaseRanges());
  await expect(border).not.toHaveAttribute("title");
  await border.check();
  await expect(page.getByLabel("Weed margin")).toHaveValue("3");

  await page.getByRole("button", { name: "Close" }).click();
  await page.reload();
  await page.evaluate(() => (window as unknown as { __failRanges: () => void }).__failRanges());
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await openWeeding(page);
  await expect(page.getByLabel("Weed border")).toBeDisabled();
  await expect(page.getByLabel("Weed border")).toHaveAttribute("title", /Weeding is unavailable: settings ranges could not be read/);
});

test("reopening the cut dialog keeps the weed controls as they were left", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  await page.getByLabel("Weed margin").fill("4");
  await page.getByRole("button", { name: "Close" }).click();

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByLabel("Weed border")).toBeChecked();
  await expect(page.getByLabel("Weed margin")).toHaveValue("4");
  // And the first plan of the reopened dialog is made with them.
  const preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 3 travel moves, 2 weed paths");
});

// Greptile's P1 on this PR, with its own Playwright repro: a replan that fails leaves the previous
// plan installed and cuttable, and the picker goes back to its mode - but travel was cleared on the
// way out and nothing brought it back. The operator was then offered a cut whose preview showed no
// travel at all, while the cut itself would travel exactly as before. The preview's accessible name
// is what makes the two states tellable apart from outside.
test("a rejected replan keeps the travel it was showing", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  // Two passes means one move between them - the travel this test is about.
  let preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");
  await goToSendStep(page, 2);

  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByLabel("Group passes by").selectOption("Single");

  // The plan failed, so the previous one is still in force: same rows, same mode, still cuttable.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByLabel("Group passes by")).toHaveValue("Color");
  preview = await showPreview(page);
  await expectStartOffered(page);
  // ...and the preview still describes the arrangement that cut would use.
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");
});

// Codex's finding on the first fix for the test above: restoring travel from a value captured when
// the replan started only works for one replan. A second replan beginning *while* the first is still
// out captures the already-cleared travel and restores that empty value on failure - so the preview
// went empty anyway, while the plan that was never replaced stayed installed and cuttable. Travel
// now lives in the installed plan, so a failure has nothing to restore and cannot lose it.
//
// The picker is disabled during a replan, so the two overlapping replans arrive the way an operator
// would actually produce them: through the stale-plan banner, whose Replan is deliberately still
// pressable - the banner is how a wedged plan gets unwedged, and a second press must not make
// things worse than the first.
test("a replan failing while another is parked keeps the installed plan's travel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  let preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");

  // Stale the plan so the banner - and its Replan - are on screen.
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
      "commit_transform",
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
    ),
  );
  await page.getByRole("button", { name: "Start Cut" }).click();
  await expect(page.getByText("Your design changed.")).toBeVisible();

  // First Replan is parked in flight: the window in which travel used to be cleared.
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByRole("button", { name: "Replan" }).click();

  // Second Replan, pressed inside that window, fails at once - `failNextPlan` is read before the
  // hold. Under the old design its rollback value was the cleared travel, not the installed one.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByRole("button", { name: "Replan" }).click();
  await expect(page.getByText(/no fonts are installed/)).toBeVisible();

  // Nothing was ever replaced, so the original plan is still the installed one - and the preview
  // must still describe it rather than showing a cut with no travel at all.
  await goToSendStep(page, 2);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");

  // The parked reply lands last and is superseded, so it changes nothing either.
  await goToSendStep(page, 2);
  await page.evaluate(() => (window as unknown as { __releasePlans: () => void }).__releasePlans());
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");
});

// Codex's finding on the fix above, and the other half of it: moving travel into the plan removed
// the need to orphan pending travel when a replan *starts*, and leaving that bump in place turned it
// into the bug. A row edit's travel reply, orphaned on the way out of a replan that then fails, never
// lands - and the plan keeps the edited rows, so one enabled pass is left showing the travel of two,
// permanently. Orphaning belongs at installation, which is the one moment the rows a reply was
// computed for stop being the rows on screen.
test("a travel reply owed to a row edit still lands when a replan fails", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  let preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 2 passes, 1 travel move");
  await goToSendStep(page, 2);

  // Disable a pass and park the travel reply it asks for: one pass left to cut means no travel
  // between passes, which is the answer this plan is owed.
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByTestId("cut-pass-row").first().getByRole("checkbox").uncheck();
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 1 travel move");
  await goToSendStep(page, 2);

  // Now a replan fails. The edited rows stay installed - so the parked reply is still the right
  // answer for them, and discarding it would leave the count above standing for good.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByLabel("Group passes by").selectOption("Single");
  await expect(page.getByText(/no fonts are installed/)).toBeVisible();
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.evaluate(() => (window as unknown as { __releaseTravel: () => void }).__releaseTravel());
  preview = await showPreview(page);
  await expect(preview).toHaveAccessibleName("Cut preview: 1 pass, 0 travel moves");
});

// The whole stale-material path through the real UI: the properties panel assigns a listed
// user preset, the dialog deletes it, preset grouping keeps the document's id on the pass, and
// the cut is refused because that id no longer resolves. Without this, `unknown_preset` could
// disappear while every saved-preset cut still passed.
test("a pass whose assigned preset was deleted is refused, not cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.goto("/");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await expect(page.getByLabel("Material preset")).toBeVisible();
  await showTab(page, "Properties");
  await page.getByLabel("Material preset").selectOption("preset:card-stock");
  await showTab(page, "Properties");
  await expect(page.getByLabel("Material preset")).toHaveValue("preset:card-stock");

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPresetEditor(page);
  await page.getByLabel("Group passes by").selectOption("Preset");
  // One pass for the assigned material, one for everything that resolves to none.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  // The pass resolves while the preset exists, and keeps the document's id once it is gone.
  await expect(page.getByTestId("cut-pass-row").first()).toContainText("Card Stock");
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Delete preset").click();
  await expect(page.getByTestId("cut-pass-row").first()).toContainText("card-stock (unknown preset)");

  await startCut(page);
  await expect(page.getByText(/not available for this machine/)).toBeVisible();
});

// The same refusal for the one id that reads like no id at all. Nothing else pins the fake's
// explicit null check: were it truthiness, an empty id would be cut with defaults here while
// `prepare_cut` refused it — a green e2e for a cut the real backend rejects.
test("an empty preset id is still named and refused", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedEmptyPresetAssignment: true,
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await page.getByLabel("Group passes by").selectOption("Preset");

  // Copilot on PR #272: the picker keys its options in the pass-key grammar, so the pass keyed on a
  // preset called `""` selects that preset rather than "No preset" — which a bare-id picker could
  // not express, since it had to spend the empty string on the absence.
  await expect(page.getByLabel("Preset for pass 1")).toHaveValue("preset:");
  await expect(page.getByTestId("cut-pass-row").first()).toContainText("(unknown preset)");
  await startCut(page);
  await expect(page.getByText(/material preset ``.*not available for this machine/)).toBeVisible();
});

/** A `cut` sent straight at the fake, carrying the plan's own revision unless the caller wants a
 *  stale one, with the refusal returned rather than thrown. These are requests the dialog cannot
 *  compose — it never sends an unplanned key, another machine's preset, or no `preset_id` field —
 *  so only a direct call can state what the backend would answer for one. `Single` plans one pass
 *  keyed `all`, which is what the callers below name. */
const cutDirect = (
  page: Page,
  request: { doc_revision?: string; grouping: string; passes: Record<string, unknown>[] },
) =>
  page.evaluate(async (req) => {
    // The fake's own channel, as everywhere else in this file: `__TAURI_INTERNALS__` is installed
    // by the fake, so the page's types do not know it.
    const internals = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
    };
    const plan = await internals.__TAURI_INTERNALS__.invoke("plan_cut", { grouping: req.grouping }) as {
      doc_revision: string;
    };
    try {
      return await internals.__TAURI_INTERNALS__.invoke("cut", {
        request: {
          device_instance_id: "usb:mock",
          doc_revision: req.doc_revision ?? plan.doc_revision,
          grouping: req.grouping,
          passes: req.passes,
        },
      });
    } catch (reason) {
      return reason;
    }
  }, request);

// Precedence: this request is stale *and* names a pass no plan has *and* names a missing preset.
// `no-preset` is a key the grammar accepts and a `Single` plan does not contain, so the payload is
// one the backend would really deserialize — a key like `missing` would die in `PassKey`'s parser
// before `prepare_cut` ran, and prove nothing about the order of its refusals. Were the preset
// check to drift below them, `stale_plan` would answer here while every UI test stayed green.
test("an unavailable preset is refused before later cut request checks", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();

  const error = await cutDirect(page, {
    doc_revision: "stale",
    grouping: "Single",
    passes: [{ key: "no-preset", enabled: true, preset_id: "gone", speed: null, force: null, repeat_count: null }],
  });
  expect(error).toMatchObject({ code: "unknown_preset" });
});

// A preset is machine-scoped, and `prepare_cut` filters the file to the connected cutter before it
// looks an id up — an operator's id is their own string, so the same one names different materials
// on two machines (#153). `puma-htv` is a material this Cameo cannot offer even though it is a
// listed builtin, which is what fails if the cut path ever resolves against every machine's
// entries: nothing else here would notice, since the tests above delete the entry outright.
test("a preset belonging to another cutter is not available to this one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();

  const error = await cutDirect(page, {
    grouping: "Single",
    passes: [{ key: "all", enabled: true, preset_id: "puma-htv", speed: null, force: null, repeat_count: null }],
  });
  expect(error).toMatchObject({
    code: "unknown_preset",
    message: expect.stringContaining("`puma-htv`"),
  });
});

// The other half of the comparison the guard spells out: `ConfiguredPassDto::preset_id` is an
// `Option<String>`, so serde reads a pass carrying no such field as `None` and the cut proceeds. A
// fake that refused it would fail a request the backend accepts — the mirror wrong in the strict
// direction, which no UI test can reach because the dialog always sends the field.
test("a pass that carries no preset id at all is cut, not refused", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();

  const outcome = await cutDirect(page, {
    grouping: "Single",
    passes: [{ key: "all", enabled: true, speed: null, force: null, repeat_count: null }],
  });
  expect(outcome).toMatchObject({ job_id: expect.any(Number) });
});

// --- managing the operator's own material presets, in the dialog that cuts with them (#244) ---
//
// Each of these drives the real editor against the fake's preset store, which mirrors every
// refusal `desktop::device::save_preset` and `delete_preset` make. The invariants are the ones a
// unit test cannot reach: what is written, what comes back, and what is selected afterwards.

/** The fake's own hooks, reached through the channel the app itself invokes over. The cast is
 *  named here rather than repeated inside a callback: `__TAURI_INTERNALS__` is installed by the
 *  fake, so nothing in the page's own types knows about it. */
const callFake = (page: Page, cmd: string) =>
  page.evaluate((name) => {
    const internals = window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } };
    return internals.__TAURI_INTERNALS__.invoke(name);
  }, cmd);

const callFakeWith = (page: Page, cmd: string, args: Record<string, unknown>) =>
  page.evaluate(({ name, payload }) => {
    const internals = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
    };
    return internals.__TAURI_INTERNALS__.invoke(name, payload);
  }, { name: cmd, payload: args });

/** Opens the Cut dialog on the local Cameo, which is the machine the presets below belong to. */
const openDialogOnCameo = async (page: Page) => {
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPresetEditor(page);
};

test("a preset created in the cut dialog is offered to a pass and cut by its id", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Thick Card");
  await page.getByLabel("Preset speed").fill("7");
  await page.getByLabel("Preset force", { exact: true }).fill("21");
  await page.getByLabel("Preset repeat count").fill("2");
  await page.getByLabel("Save preset", { exact: true }).click();

  // Selected on what came back from the backend, not on what was typed: the file is what a preset
  // is, and the editor re-reads it after every write.
  await expect(page.getByLabel("Preset to manage")).toHaveValue("thick-card");
  await expect(page.getByTestId("preset-preview")).toHaveText("Cuts at speed 7, force 21, 2 passes.");
  // And a pass can now be cut with it, which is the whole point of managing them here.
  await expect(page.getByLabel("Preset for pass 1").locator("option")).toContainText(["No preset", "HTV", "Thick Card"]);
  await page.getByLabel("Preset for pass 1").selectOption("preset:thick-card");
  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();

  // Which pass carries it, not merely that some pass does: the dialog attaching the preset to the
  // wrong row is the regression this is here for, and the untouched row must still name none.
  const request = await callFake(page, "__test_last_cut_request") as {
    passes: { key: string; enabled: boolean; preset_id: string | null }[];
  };
  expect(request.passes.map((p) => [p.key, p.preset_id])).toEqual([
    ["color:ff0000ff", "thick-card"],
    ["color:00ff00ff", null],
  ]);
});

test("a built-in preset is read-only, and Save as Copy leaves it shipped", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("Preset to manage").selectOption("cameo5-htv");
  await expect(page.getByText("built-in — read-only")).toBeVisible();
  await expect(page.getByLabel("Preset name")).toBeDisabled();
  await expect(page.getByLabel("Delete preset")).toHaveCount(0);

  await page.getByLabel("Save as Copy").click();
  // A fresh id under a name of its own: an entry saved under `cameo5-htv` would shadow the shipped
  // material, and the backend refuses that pair outright.
  await expect(page.getByLabel("Preset to manage")).toHaveValue("htv-copy");
  await expect(page.getByLabel("Preset name")).toHaveValue("HTV (copy)");
  await expect(page.getByLabel("Preset name")).toBeEnabled();
  // The builtin is still listed, and still a builtin.
  await page.getByLabel("Preset to manage").selectOption("cameo5-htv");
  await expect(page.getByText("built-in — read-only")).toBeVisible();
  await expect(page.getByTestId("preset-preview")).toHaveText("Cuts at speed 5, force 20, one pass.");
});

test("renaming a preset keeps the id a pass and a document name it by", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Thick Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("thick-card");

  await page.getByLabel("Preset name").fill("Thin Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  // The name moved; the id did not. A PassKey is `preset:<id>` and a Node's assignment names the
  // same string, so an id that followed the name would orphan every document holding it.
  await expect(page.getByLabel("Preset to manage")).toHaveValue("thick-card");
  await expect(page.getByLabel("Preset to manage").locator("option")).toContainText(["HTV (built-in)", "Thin Card"]);
});

test("a name this cutter already uses is refused before anything is written", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("HTV");
  await expect(page.getByTestId("preset-error")).toContainText("built-in");
  await expect(page.getByLabel("Save preset", { exact: true })).toBeDisabled();

  // A name of its own clears it, and the same press then writes.
  await page.getByLabel("Preset name").fill("HTV, mine");
  await expect(page.getByTestId("preset-error")).toHaveCount(0);
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("htv-mine");
});

// CodeRabbit on the second push: the write and the re-read that follows it were caught together, so
// a read that failed after a write that did not reported a refused save, kept the draft as unsaved,
// and swallowed whatever was waiting on it.
test("a write that landed is not reported as refused when the re-read after it fails", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("card");

  // The save lands and the list read behind it does not: the section says the list could not be
  // read. What proves it was not read as a refused save is the draft — a refusal keeps it unsaved,
  // and the next guarded action would be parked to ask about it. This Close is not parked
  // (CodeRabbit: asserting on `preset-error` cannot fail here, since the editor is not rendered at
  // all while the list is unavailable).
  await page.getByLabel("Preset force", { exact: true }).fill("18");
  await callFake(page, "__test_fail_next_preset_list");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByText("Material presets are unavailable")).toContainText("could not be read");
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // And 18 is in the file: a fresh dialog reads the list again — no Connect, the cutter is still
  // connected — and the failure the section was showing is answered by that read.
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPresetEditor(page);
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("force 18");

  // The same interleaving under the unsaved-changes decision: Save and continue writes, the read
  // behind it fails, the close it was blocking still happens — and 19 reaches the file too.
  await page.getByLabel("Preset force", { exact: true }).fill("19");
  await callFake(page, "__test_fail_next_preset_list");
  await page.getByRole("button", { name: "Close" }).click();
  await page.getByLabel("Save preset and continue").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPresetEditor(page);
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("force 19");
});

test("deleting a preset selects a neighbour instead of showing settings that are gone", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  for (const name of ["Card", "Vinyl"]) {
    await page.getByLabel("New preset").click();
    await page.getByLabel("Preset name").fill(name);
    await page.getByLabel("Save preset", { exact: true }).click();
    await expect(page.getByLabel("Preset name")).toHaveValue(name);
  }

  await page.getByLabel("Preset to manage").selectOption("card");
  await page.getByLabel("Delete preset").click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("vinyl");
  await expect(page.getByLabel("Preset to manage").locator("option")).toContainText(["HTV (built-in)", "Vinyl"]);
  await expect(page.getByLabel("Preset to manage").locator("option")).toHaveCount(2);
});

// Codex on the third push: the write's own re-read reselects what it just saved, so a Save and
// continue whose continuation was "show me that other preset" landed on the saved one instead — the
// operator's next act undone by the write they asked for. Every existing decision test continued
// into a Close, where the dialog unmounts and the overwrite cannot be seen.
test("save and continue lands on the preset the operator asked for, not the one just written", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  for (const name of ["Card", "Vinyl"]) {
    await page.getByLabel("New preset").click();
    await page.getByLabel("Preset name").fill(name);
    await page.getByLabel("Save preset", { exact: true }).click();
    await expect(page.getByLabel("Preset name")).toHaveValue(name);
  }

  await page.getByLabel("Preset to manage").selectOption("card");
  await page.getByLabel("Preset speed").fill("14");
  await page.getByLabel("Preset to manage").selectOption("vinyl");
  await page.getByLabel("Save preset and continue").click();

  // Vinyl is what was asked for, and Card holds the 14 that was saved on the way there.
  await expect(page.getByLabel("Preset to manage")).toHaveValue("vinyl");
  await expect(page.getByLabel("Preset name")).toHaveValue("Vinyl");
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 14");
});

test("a write the backend refuses keeps the edit on screen and says why", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Thick Card");
  await page.getByLabel("Preset force", { exact: true }).fill("30");
  await callFake(page, "__test_fail_next_preset_save");
  await page.getByLabel("Save preset", { exact: true }).click();

  // The refusal is named, and the numbers are still there to try again with — they exist nowhere
  // else, so a cleared form would be the operator retyping them from memory.
  await expect(page.getByTestId("preset-error")).toContainText("presets file could not be written");
  await expect(page.getByLabel("Preset name")).toHaveValue("Thick Card");
  await expect(page.getByLabel("Preset force", { exact: true })).toHaveValue("30");

  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("thick-card");
  await expect(page.getByTestId("preset-error")).toHaveCount(0);
});

test("an unsaved preset edit has to be decided before the dialog closes", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Preset speed").fill("9");
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByText("Unsaved changes to the preset")).toBeVisible();

  // Keep editing leaves them exactly where they were, dialog and draft alike.
  await page.getByLabel("Keep editing the preset").click();
  await expect(page.getByLabel("Preset speed")).toHaveValue("9");

  // Save and continue does both: the entry is written, and then the close it was blocking happens.
  await page.getByRole("button", { name: "Close" }).click();
  await page.getByLabel("Save preset and continue").click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // Reopening proves the save reached the backend rather than the dialog's own memory. No Connect
  // this time: the cutter is still connected, and the dialog seeds itself from the manager's cache
  // — pressing the first Connect on screen would aim at the *other* local cutter.
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPresetEditor(page);
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 9");
});

test("discarding an unsaved edit writes nothing and lets the interrupted action through", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("card");

  await page.getByLabel("Preset speed").fill("12");
  // Selecting another preset is a discard just as closing is, so it is asked about the same way.
  await page.getByLabel("Preset to manage").selectOption("cameo5-htv");
  await expect(page.getByText("Unsaved changes to the preset")).toBeVisible();
  await page.getByLabel("Discard preset changes and continue").click();

  await expect(page.getByText("built-in — read-only")).toBeVisible();
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed from the cutter's panel");
});

test("a preset belongs to one cutter: aiming at another shows that machine's entries", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("My Vinyl");
  await page.getByLabel("Preset speed").fill("7");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("my-vinyl");

  // Aiming at a second local cutter means letting go of the first: `DeviceManager` refuses a
  // Connect while it holds a transport, so the dialog's Disconnect is the way across. The Puma's
  // list is its own — an id is the operator's own string, so one machine's entry must not appear,
  // or be editable, under another (#153).
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("radio", { name: "GCC Puma IV", exact: true }).click();
  await goToSendStep(page, 2);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "HTV (built-in)" })).toHaveCount(1);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "My Vinyl" })).toHaveCount(0);

  // And the Cameo's entry is untouched by the trip, settings and all.
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect serial:/dev/mock0").click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await goToSendStep(page, 2);
  await page.getByLabel("Preset to manage").selectOption("my-vinyl");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 7");
});

// Greptile's P1 on the first push: a write leaves a `list_presets` out, and its reply installed the
// list *and* re-derived the draft from it with nothing asking which cutter it was read for. Aim at
// another cutter in that window and the editor showed the previous machine's entry — and a save
// would then have written that draft under the new machine's id.
//
// Reached through a delete rather than a save: a save leaves the draft dirty until its reply lands,
// so the unsaved-changes guard holds the cutter change back. A delete leaves nothing unsaved, so
// the aim really can move while the list is still out.
test("a preset list still owed to the previous cutter is not shown against this one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  for (const name of ["My Vinyl", "Card"]) {
    await page.getByLabel("New preset").click();
    await page.getByLabel("Preset name").fill(name);
    await page.getByLabel("Save preset", { exact: true }).click();
    await expect(page.getByLabel("Preset name")).toHaveValue(name);
  }

  // Park every list reply, delete the Cameo's entry, then let go of the Cameo and aim at the Puma
  // while that list is still out — the sequence production allows, since a Connect is refused while
  // the manager holds a transport. The Puma's own read parks behind it, so both land on release.
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Preset to manage").selectOption("my-vinyl");
  await page.getByLabel("Delete preset").click();
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("radio", { name: "GCC Puma IV", exact: true }).click();
  await goToSendStep(page, 2);
  await callFake(page, "__test_release_presets");

  // The Cameo's reply is inert: no draft of its material, and none of its entries in the picker.
  await expect(page.getByText("Choose a preset to edit")).toBeVisible();
  await expect(page.getByLabel("Preset name")).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "Card" })).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "HTV (built-in)" })).toHaveCount(1);

  // And the delete that was in flight did reach the Cameo's file, where it belonged.
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect serial:/dev/mock0").click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await goToSendStep(page, 2);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "My Vinyl" })).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "Card" })).toHaveCount(1);
});

// Round 6 on PR #264: three findings, one cause. Every guard above was keyed on the machine id, and
// a machine id is not an aim — two aims at the same cutter share it. So a list read for a previous
// connection installed as though it were this one's (Copilot), and a continuation captured under
// that aim restored its entry as this aim's draft, which the next save wrote under this machine's id
// (Greptile, P1). The section is keyed on an aim generation now, the way the plan and the travel
// already are.
test("a preset list owed to a previous connection to the same cutter is not taken for this one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Preset to manage")).toHaveValue("card");

  // Park every list reply, then save — that write's own re-read is now owed to *this* connection —
  // and let go of the Cameo and aim at it again. Both replies name the same machine, so nothing but
  // the aim tells them apart, and the older one would re-derive the draft it was written for.
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Preset speed").fill("16");
  await page.getByLabel("Save preset", { exact: true }).click();
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await goToSendStep(page, 2);

  // This aim has read nothing yet: the editor is withheld, whatever the previous connection left
  // behind. Rendered from that cached list, New would mint an id against entries nobody re-read.
  await expect(page.getByText("Reading this cutter's presets…")).toBeVisible();
  await expect(page.getByLabel("New preset")).toHaveCount(0);

  // Released together, oldest first. The previous connection's reply is inert: the editor opens on
  // this aim's list with nothing selected, rather than back on the entry that write had settled.
  await callFake(page, "__test_release_presets");
  await expect(page.getByLabel("New preset")).toBeEnabled();
  await expect(page.getByText("Choose a preset to edit")).toBeVisible();
  await expect(page.getByLabel("Preset name")).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "Card" })).toHaveCount(1);

  // And the save that was in flight did land, so what is inert is the reply, not the write.
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 16");
});

test("a save's continuation is dropped when the cutter changed before it could run", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  for (const name of ["Card", "Vinyl"]) {
    await page.getByLabel("New preset").click();
    await page.getByLabel("Preset name").fill(name);
    await page.getByLabel("Save preset", { exact: true }).click();
    await expect(page.getByLabel("Preset name")).toHaveValue(name);
  }

  // Dirty on Card, ask for Vinyl, and answer Save and continue with the list replies parked: the
  // write lands, the continuation is still owed, and the aim moves to the Puma before it can run.
  await page.getByLabel("Preset to manage").selectOption("card");
  await page.getByLabel("Preset speed").fill("13");
  await page.getByLabel("Preset to manage").selectOption("vinyl");
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Save preset and continue").click();
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("radio", { name: "GCC Puma IV", exact: true }).click();
  await goToSendStep(page, 2);
  await callFake(page, "__test_release_presets");

  // Nothing of the Cameo's arrives on the Puma: no draft, and no Cameo entry in its picker. The
  // continuation named Vinyl by the Cameo's list, and run here it would have become the Puma's
  // draft — then the Puma's entry under the next save.
  await expect(page.getByText("Choose a preset to edit")).toBeVisible();
  await expect(page.getByLabel("Preset name")).toHaveCount(0);
  await expect(page.getByLabel("Preset to manage").locator("option", { hasText: "Vinyl" })).toHaveCount(0);

  // And the save that carried the continuation did land on the Cameo, where it was aimed.
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect serial:/dev/mock0").click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await goToSendStep(page, 2);
  await page.getByLabel("Preset to manage").selectOption("card");
  await expect(page.getByTestId("preset-preview")).toContainText("speed 13");
});

// Codex's findings on the second push, both about an action that replaces the draft without the
// unsaved-changes decision: Duplicate writes from the *stored* entry (so it would drop the edit, or
// copy a version that no longer exists), and Delete replaces the draft with a neighbour's. Neither
// is offered while there is an edit to lose. The editor itself is withheld until the aimed cutter's
// own list has arrived, because that list is what a new entry's name and id have to avoid.
test("an unsaved edit withholds the actions that would discard it, and an unread list withholds the editor", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByLabel("Duplicate preset")).toBeEnabled();
  await expect(page.getByLabel("Delete preset")).toBeEnabled();

  await page.getByLabel("Preset force", { exact: true }).fill("18");
  await expect(page.getByLabel("Duplicate preset")).toBeDisabled();
  await expect(page.getByLabel("Delete preset")).toBeDisabled();

  // Discarding is one of the two ways back, and both come back at once.
  await page.getByLabel("Discard preset changes").click();
  await expect(page.getByLabel("Duplicate preset")).toBeEnabled();
  await expect(page.getByLabel("Delete preset")).toBeEnabled();

  // With every list reply parked, aiming at the Puma leaves nothing to create against: no picker,
  // no New, and a line saying why.
  await callFake(page, "__test_hold_presets");
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect usb:mock").click();
  await page.getByRole("radio", { name: "GCC Puma IV", exact: true }).click();
  await goToSendStep(page, 2);
  await expect(page.getByText("Reading this cutter's presets…")).toBeVisible();
  await expect(page.getByLabel("New preset")).toHaveCount(0);

  await callFake(page, "__test_release_presets");
  await expect(page.getByLabel("New preset")).toBeEnabled();
});

// The pass rows' half of the same window, and the one #267 was filed for: the editor above is
// withheld until this aim's list arrives, but the rows are rendered from it regardless, and they
// name a material by looking its id up in exactly that list. Read as an ordinary empty list, a row
// tells the operator their pass has no material while `prepare_cut` would resolve it from the
// presets file and cut it.
test("a pass whose preset list has not arrived is named as unread, not as having no material", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.goto("/");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await page.getByLabel("Material preset").selectOption("preset:card-stock");

  // Parked before the connect that asks for it, so every row below is rendered against a list
  // nobody has answered for — the seconds after a connect, held open.
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await callFake(page, "__test_hold_presets");
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await goToSendStep(page, 2);
  await expect(page.getByText("Reading this cutter's presets…")).toBeVisible();
  await page.getByLabel("Group passes by").selectOption("Preset");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  const row = page.getByTestId("cut-pass-row").first();
  await expect(row).toContainText("card-stock (name unread)");
  // The picker carries the pass's own preset rather than nothing: a `select` whose value matches no
  // option renders blank, and blank is exactly what "No preset" looks like.
  await expect(page.getByLabel("Preset for pass 1")).toHaveValue("preset:card-stock");
  // And the repeat says nothing rather than one pass, which is a claim about the blade.
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  // The unread preset is not a blank the form can fill. The other pass names no preset, and
  // that blank still holds Next until it is set — filling it is what expectStartOffered does.
  await expect(page.getByLabel("Speed for pass 1")).not.toHaveAttribute("aria-invalid", "true");
  await expect(page.getByLabel("Speed for pass 2")).toHaveAttribute("aria-invalid", "true");
  await expect(page.getByText("Use 1–30").first()).toBeVisible();
  await expect(page.getByRole("button", { name: "Next", exact: true })).toBeDisabled();
  // Cutting stays available once that blank is filled: the backend resolves the material from
  // the presets file, and refuses by name when it cannot.
  await expectStartOffered(page);

  // The name and its settings arrive together, under the aim that asked for them.
  await callFake(page, "__test_release_presets");
  await expect(row).toContainText("Card Stock");
  await expect(page.getByLabel("Preset for pass 1")).toHaveValue("preset:card-stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("1");

  // Greptile's P1 on the second push: disconnecting clears the list with the aim, and presets are
  // machine-scoped, so from here nothing can resolve the name — not "reading", which is why the
  // marker does not say so. What must not come back is `(unknown preset)`: the material is not gone,
  // and the file the backend would resolve it from is untouched by a disconnect.
  await goToSendStep(page, 1);
  await page.getByLabel("Disconnect usb:mock").click();
  await goToSendStep(page, 2);
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("unknown preset");
  await expect(page.getByLabel("Preset for pass 1")).toHaveValue("preset:card-stock");
});

// #274, the sharper half of the same family: the list a row prices from is replaced only by a read
// that succeeds, so between a write landing and its re-read arriving — and forever if that read
// fails — the row names and prices the pass from the entry as it was *before* the write, while
// `prepare_cut` resolves the new one from the presets file. A stale number reads as a fact.
test("a written preset's previous name and settings leave the pass row when the write lands", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.goto("/");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await page.getByLabel("Material preset").selectOption("preset:card-stock");

  await openDialogOnCameo(page);
  await page.getByLabel("Group passes by").selectOption("Preset");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  const row = page.getByTestId("cut-pass-row").first();
  await expect(row).toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("1");

  // Rename the material the row is priced from and make it cut three times, with every list reply
  // parked: the write lands, so that is what the file holds and what the blade will do, and the
  // read that would say so is still out.
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Preset name").fill("Heavy Card");
  await page.getByLabel("Preset repeat count").fill("3");
  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Save preset", { exact: true }).click();

  // Neither the name nor the repeat the entry used to have. Unread is what the row has: an answer
  // is owed and none has arrived.
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  // Cutting stays available throughout: the backend resolves the material from the presets file,
  // and refuses by name when it cannot, so nothing here is a gate.
  await expectStartOffered(page);
  // And the editor above the rows does not go with them. It is withheld only until *some* list for
  // this aim has arrived, because what is stored is what a new entry's id and name must avoid —
  // withholding it here would take it off screen on every save.
  await expect(page.getByLabel("Preset name")).toHaveValue("Heavy Card");

  await callFake(page, "__test_release_presets");
  await expect(row).toContainText("Heavy Card");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("3");

  // The half that outlives the window: a re-read that fails never replaces the list, so the row
  // must not fall back to the entry the write superseded — which it did for the life of the dialog.
  await page.getByLabel("Preset repeat count").fill("5");
  await callFake(page, "__test_fail_next_preset_list");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(page.getByText(/Material presets are unavailable/)).toBeVisible();
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("Heavy Card");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  await expectStartOffered(page);
});

// The delete half of the same window, which is its own branch of the dialog and can regress on its
// own (Copilot on PR #275): a delete lands in the presets file exactly as a save does, so the list
// held from before it is just as superseded — and a row priced from it would name a material the
// operator has just thrown away.
test("a deleted preset's name and settings leave the pass row when the delete lands", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.goto("/");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await page.getByLabel("Material preset").selectOption("preset:card-stock");

  await openDialogOnCameo(page);
  await page.getByLabel("Group passes by").selectOption("Preset");
  const row = page.getByTestId("cut-pass-row").first();
  await expect(row).toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("1");

  await callFake(page, "__test_hold_presets");
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Delete preset").click();

  // Unread, not "unknown preset": the delete has landed but nothing has read the file back, so the
  // row cannot yet say the material is gone — only that it has no answer about it.
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  await expectStartOffered(page);
  // The editor stays, for the same reason it stays across a save: it is what the held entries are
  // kept for.
  await expect(page.getByLabel("Preset to manage")).toBeVisible();

  // Once a list does arrive, the row may say what it now knows — the document names a preset this
  // cutter no longer has — and prices the pass at the default single pass.
  await callFake(page, "__test_release_presets");
  await expect(row).toContainText("card-stock (unknown preset)");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("1");

  // Re-mint the entry the document names, so the failing window has a name and a repeat count of
  // its own to leave behind rather than the ones already proven above.
  await page.getByLabel("New preset").click();
  await page.getByLabel("Preset name").fill("Card Stock");
  await page.getByLabel("Preset repeat count").fill("4");
  await page.getByLabel("Save preset", { exact: true }).click();
  await expect(row).toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("4");

  // And a re-read that fails after a delete leaves the row unread for good, rather than falling
  // back to the entry the delete removed.
  await callFake(page, "__test_fail_next_preset_list");
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Delete preset").click();
  await expect(page.getByText(/Material presets are unavailable/)).toBeVisible();
  await expect(row).toContainText("card-stock (name unread)");
  await expect(row).not.toContainText("Card Stock");
  await expect(page.getByLabel("Repeat count for pass 1")).toHaveValue("");
  await expectStartOffered(page);
});

test("the whole editor is operable from the keyboard alone", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await openDialogOnCameo(page);

  // Focus, then keys only: nothing below is a div with a click handler, which is the failure this
  // catches — an operator with the keyboard and no mouse can still reach every control.
  await page.getByLabel("New preset").focus();
  await page.keyboard.press("Enter");
  await page.getByLabel("Preset name").focus();
  await page.keyboard.type("Keyed Card");
  // Tab reaches the settings in the order they are read.
  await page.keyboard.press("Tab");
  await page.keyboard.type("8");
  await page.keyboard.press("Tab");
  await page.keyboard.type("22");
  await page.getByLabel("Save preset", { exact: true }).focus();
  await page.keyboard.press("Enter");

  await expect(page.getByLabel("Preset to manage")).toHaveValue("keyed-card");
  await expect(page.getByTestId("preset-preview")).toHaveText("Cuts at speed 8, force 22, one pass.");
});

// Greptile's P1 on the fifth push: a replan that *fails* leaves the previous plan in force —
// rows, revision and mode — but the picker had already moved to the mode nobody managed to plan.
// Cut then sent the old grouping while the operator read the new one off the screen. Nothing
// miscuts, which is what makes it worth a test: the lie is only visible on the dialog.
test("a grouping whose plan fails does not stay on the picker", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByLabel("Group passes by")).toHaveValue("Color");

  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_plan"));
  await page.getByLabel("Group passes by").selectOption("Single");

  // The refusal is reported, the previous plan is still what would be cut, and the picker says
  // so rather than advertising the mode that failed.
  await expect(page.getByText(/no fonts are installed/)).toBeVisible();
  await expect(page.getByLabel("Group passes by")).toHaveValue("Color");
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
});

// Greptile's P1 on PR #152, reproduced with a held reply: while a replacement plan is in flight
// the rows on screen still belong to the previous grouping, so an edit accepted there is
// discarded when the new plan installs — silently, with the operator's speed still on screen
// until it vanishes. Every row control is unavailable in that window.
test("row controls are unavailable while a replacement plan is in flight", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByLabel("Speed for pass 1")).toBeEnabled();

  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await page.getByLabel("Group passes by").selectOption("Single");

  // Still the old mode's two rows, and not one of their controls will take an edit.
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByLabel("Speed for pass 1")).toBeDisabled();
  await expect(page.getByLabel("Force for pass 1")).toBeDisabled();
  await expect(page.getByLabel("Repeat count for pass 1")).toBeDisabled();
  await expect(page.getByLabel("Preset for pass 1")).toBeDisabled();
  await expect(page.getByRole("checkbox", { name: "Enabled" }).first()).toBeDisabled();
  await expect(page.getByRole("button", { name: "Reorder pass 1" })).toBeDisabled();

  await page.evaluate(() => (window as unknown as { __releasePlans: () => void }).__releasePlans());
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  await expect(page.getByLabel("Speed for pass 1")).toBeEnabled();
});

// The whole operator-facing round trip: the properties panel's control, the real
// set_cut_line_type command, and the plan that then leaves the shape out. Nothing else in this
// suite reads the dialog's not-cut line, so a readout wired to a renamed field would render an
// `undefined` and every other test would still pass — hence the assertion on its full text,
// count and reason both. It is also the only exercise of planFromDoc's NoCut branch.
test("marking a shape No Cut drops its pass, and the dialog says why", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  const cuttable = page.getByRole("checkbox", { name: "Cut this shape" });
  await expect(cuttable).toBeChecked();
  await cuttable.uncheck();

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(1);
  await expect(page.getByText("Not cut: 1 shape marked No Cut")).toBeVisible();

  // Discriminating step: a command that ignored `value` and only ever wrote NoCut would pass
  // everything above. Marking it back has to restore the pass and empty the readout.
  await page.getByRole("button", { name: "Close" }).click();
  await showTab(page, "Properties");
  await cuttable.check();
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByText("Not cut: 0 shapes marked No Cut")).toBeVisible();

  // A selection whose shapes disagree is the one state the panel cannot answer with a plain
  // tick, and the direction it picks when clicked is not cosmetic: `checked={cutLineType !==
  // "NoCut"}` would render mixed as *checked*, so the click commits NoCut across the whole
  // selection and shapes silently stop cutting — with every other assertion here still green.
  await page.getByRole("button", { name: "Close" }).click();
  await cuttable.uncheck();
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(1).click({ modifiers: ["Shift"] });
  await showTab(page, "Properties");
  await expect(cuttable).toBeChecked({ indeterminate: true });

  await cuttable.click();
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);
  await expect(page.getByText("Not cut: 0 shapes marked No Cut")).toBeVisible();
});

// The plan is made from the document, so an edit after planning must refuse the cut —
// cutplan::plan_cut's stale-plan rule. `commit_transform` is the discriminating edit:
// it changes geometry without adding or removing a node, so a revision that tracks
// commands rather than the document itself cuts stale geometry and still passes.
test("a doc edited after planning refuses the cut until replan", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  // Reaches past the UI on purpose: the canvas drag that would edit the document (through
  // commit_transforms) is behind the open dialog, and the backend contract under test, geometry
  // that changes with no node added or removed, is the same either way.
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
      "commit_transform",
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
    ),
  );

  await startCut(page);
  await expect(page.getByText("Your design changed.")).toBeVisible();

  // Stale blocks the next Start. It does not freeze the order: the grip stays usable and the
  // move still lands, which is what a later Replan has to carry.
  await goToSendStep(page, 2);
  await expect(page.getByRole("button", { name: "Reorder pass 1" })).toBeEnabled();
  const chip = (i: number) => page.getByTestId("cut-pass-row").nth(i).locator("span").first();
  await expect(chip(0)).toHaveCSS("background-color", "rgb(255, 0, 0)");
  await reorderPass(page, 0, 1);
  await expect(chip(0)).toHaveCSS("background-color", "rgb(0, 255, 0)");
  await expect(page.getByRole("button", { name: "Reorder pass 1" })).toBeEnabled();

  await page.getByRole("button", { name: "Replan" }).click();
  // A replan starts the rows again, so the blanks a no-preset pass refuses have to be filled once more.
  await goToSendStep(page, 2);
  await fillSendLevels(page);
  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
});

test("reordering passes asks the backend for travel in the new order", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  // Seeded first-seen order is red (0xff0000ff) then green (0x00ff00ff).
  const chip = (i: number) => page.getByTestId("cut-pass-row").nth(i).locator("span").first();
  await expect(chip(0)).toHaveCSS("background-color", "rgb(255, 0, 0)");

  await reorderPass(page, 0, 1);
  await expect(chip(0)).toHaveCSS("background-color", "rgb(0, 255, 0)");
  await expect(page.getByRole("button", { name: "Up", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Down", exact: true })).toHaveCount(0);

  // The wire is the contract under test: the replan request names the swapped order.
  const requests = await page.evaluate(
    () => (window as unknown as { __travelRequests?: { key: string; enabled: boolean }[][] }).__travelRequests,
  );
  expect(requests).toEqual([[
    { key: "color:00ff00ff", enabled: true },
    { key: "color:ff0000ff", enabled: true },
  ]]);
});

test("dragging a pass asks the backend for travel in the new order", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  const chip = (i: number) => page.getByTestId("cut-pass-row").nth(i).locator("span").first();
  await expect(chip(0)).toHaveCSS("background-color", "rgb(255, 0, 0)");
  await page.getByRole("button", { name: "Reorder pass 2" }).dragTo(page.getByTestId("cut-pass-row").first());
  await expect(chip(0)).toHaveCSS("background-color", "rgb(0, 255, 0)");

  const requests = await page.evaluate(
    () => (window as unknown as { __travelRequests?: { key: string; enabled: boolean }[][] }).__travelRequests,
  );
  expect(requests).toEqual([[
    { key: "color:00ff00ff", enabled: true },
    { key: "color:ff0000ff", enabled: true },
  ]]);
});

test("a drop that is not a pass index leaves the order alone", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  const chip = (i: number) => page.getByTestId("cut-pass-row").nth(i).locator("span").first();
  await expect(chip(0)).toHaveCSS("background-color", "rgb(255, 0, 0)");
  const row = page.getByTestId("cut-pass-row").nth(1);

  for (const payload of ["foo", "1.", "", " "]) {
    await row.evaluate((target, text) => {
      const data = new DataTransfer();
      data.setData("text/plain", text);
      target.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
      target.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
    }, payload);
    await expect(row).toHaveAttribute("data-drop-payload", payload);
    await expect(chip(0)).toHaveCSS("background-color", "rgb(255, 0, 0)");
  }

  const requests = await page.evaluate(
    () => (window as unknown as { __travelRequests?: unknown[] }).__travelRequests,
  );
  expect(requests ?? []).toEqual([]);
});

test("the mini mat names the pass under the pointer and dims the others", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  const shapes = page.getByRole("img", { name: "Shapes in this pass" }).locator("rect[opacity]");
  await expect(shapes).toHaveCount(2);
  await expect(shapes.nth(0)).toHaveAttribute("opacity", "0.9");
  await expect(shapes.nth(1)).toHaveAttribute("opacity", "0.18");
  await page.getByTestId("cut-pass-row").nth(1).hover();
  await expect(shapes.nth(0)).toHaveAttribute("opacity", "0.18");
  await expect(shapes.nth(1)).toHaveAttribute("opacity", "0.9");
});

test("a machine the document was not set up for blocks the passes until it is converted", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedMachine: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  // The cutter is aimed either way. Dismissing the switch leaves the document on the Cameo, and
  // the dismissal is the fake dialog plugin's answer: the plugin raises no native dialog.
  await page.evaluate(() => sessionStorage.setItem("__dialog_answer__", "cancel"));
  await page.getByRole("radio", { name: "GCC Puma IV", exact: true }).click();
  await goToSendStep(page, 2);
  await fillSendLevels(page);
  const next = page.getByRole("button", { name: "Next", exact: true });
  await expect(page.getByText("Document is set up for a different machine than the connected device.")).toBeVisible();
  await expect(next).toBeDisabled();
  await page.getByRole("button", { name: "Convert to puma" }).click();
  await expect(page.getByText("Document is set up for a different machine than the connected device.")).toHaveCount(0);
  // Converting replans, and the new row's speed and force start blank.
  await fillSendLevels(page);
  await expect(next).toBeEnabled();
});

/**
 * The device list's own offer to retarget the document, over a document that has shapes and a
 * machine already. The answer is the prompt's, late and scripted here, because the real one is the
 * dialog plugin's — this guard used to read `window.confirm`, which the plugin replaces with an
 * async function, so Cancel switched the design anyway.
 */
for (const answer of ["cancel", "ok"] as const) {
  test(`picking the other machine ${answer === "ok" ? "switches" : "does not switch"} the design's machine`, async ({ page }) => {
    await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedMachine: true });
    await page.goto("/");
    await page.evaluate((a) => {
      sessionStorage.setItem("__dialog_answer__", a);
      sessionStorage.setItem("__dialog_hold__", "1");
    }, answer);

    await page.getByRole("tab", { name: "Devices" }).click();
    await page
      .getByRole("listitem")
      .filter({ hasText: "GCC Puma IV" })
      .getByRole("button", { name: "Set as active", exact: true })
      .click();

    // The question is raised after the connect, and the fake holds its answer until released
    // below, so what is read next is the document as it stood while the operator was being asked.
    await expect
      .poll(async () =>
        JSON.parse((await page.evaluate(() => sessionStorage.getItem("__dialog_messages__"))) ?? "[]") as string[],
      )
      .toEqual([
        "This design is set up for Silhouette Cameo 5 Alpha. Switch it to GCC Puma IV? The page size changes with the machine.",
      ]);

    const designMachine = async () =>
      (JSON.parse((await callFake(page, "snapshot")) as string) as { machine: { id: string } }).machine.id;
    expect(await designMachine()).toBe("cameo5");

    // Delivery is recorded in the same turn it resolves, so reading it afterwards is ordered
    // after the page's continuation — for Cancel there is no later change to poll for, and this
    // is what makes "still cameo5" a fact about the answer rather than about the timing.
    await page.evaluate(() => sessionStorage.setItem("__dialog_released__", "1"));
    await expect.poll(() => page.evaluate(() => sessionStorage.getItem("__dialog_answered__"))).toBe("1");
    await expect.poll(designMachine).toBe(answer === "ok" ? "puma" : "cameo5");

    // The same fact as the operator meets it: the send dialog's passes refuse a machine the
    // design was not set up for, with the dialog's own controls up before the absence is read.
    await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
    await expect(page.getByRole("button", { name: "Next", exact: true })).toBeVisible();
    const blocked = page.getByText("Document is set up for a different machine than the connected device.");
    if (answer === "cancel") {
      await expect(blocked).toBeVisible();
    } else {
      await expect(blocked).toHaveCount(0);
    }
  });
}

test("an inch weed margin is a view of the millimetre draft", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("group", { name: "Units" }).getByRole("button", { name: "in", exact: true }).click();
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  const margin = page.getByLabel("Weed margin");
  await expect(margin).toHaveValue("0.12");
  const plans = () => page.evaluate(() => (window as unknown as { __planCalls?: number }).__planCalls ?? 0);
  const before = await plans();
  // Keystrokes, not a filled value: a number input reports both `1` and `.` as "1".
  await margin.click();
  await page.keyboard.press("Control+A");
  await page.keyboard.type("1.");
  await expect(margin).toHaveValue("1.");
  await margin.blur();
  await expect(margin).toHaveValue("1.");
  await expect(page.getByRole("button", { name: "Next", exact: true })).toBeDisabled();
  expect(await plans()).toBe(before);

  await margin.fill("2");
  await expect(margin).toHaveValue("2");
  await margin.blur();
  await expect(page.getByRole("alert").filter({ hasText: "Margin must be 0.02–1.96 in" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Next", exact: true })).toBeDisabled();
  await margin.fill("1");
  await margin.blur();
  await expect(margin).toHaveValue("1.00");
  await fillSendLevels(page);
  await startCut(page);
  const request = await callFake(page, "__test_last_cut_request") as { weed: { margin_mm: number } };
  expect(request.weed.margin_mm).toBe(25.4);
});

test("escape commits a finished inch and drops a half-typed one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("group", { name: "Units" }).getByRole("button", { name: "in", exact: true }).click();
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  const margin = page.getByLabel("Weed margin");
  await expect(margin).toHaveValue("0.12");

  await margin.click();
  await page.keyboard.press("Control+A");
  await page.keyboard.type("1.");
  await expect(margin).toHaveValue("1.");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Send to cutter" })).toHaveCount(0);
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByLabel("Weed margin")).toHaveValue("0.12");

  await page.getByLabel("Weed margin").fill("1");
  await expect(page.getByLabel("Weed margin")).toHaveValue("1");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "Send to cutter" })).toHaveCount(0);
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByLabel("Weed margin")).toHaveValue("1.00");
  await fillSendLevels(page);
  await expect(page.getByRole("button", { name: "Next", exact: true })).toBeEnabled();
});

test("a margin at the millimetre maximum is shown as an inch that margin still allows", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await openWeeding(page);
  await page.getByLabel("Weed border").check();
  await page.getByLabel("Weed margin").fill("50");
  await page.getByRole("button", { name: "Close" }).click();

  await page.getByRole("group", { name: "Units" }).getByRole("button", { name: "in", exact: true }).click();
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  const margin = page.getByLabel("Weed margin");
  await expect(margin).toHaveValue("1.96");
  await margin.click();
  await page.keyboard.press("Control+A");
  await page.keyboard.type("1.96");
  await margin.blur();
  await expect(page.getByRole("alert").filter({ hasText: "Margin must be" })).toHaveCount(0);
  await fillSendLevels(page);
  await startCut(page);
  const request = await callFake(page, "__test_last_cut_request") as { weed: { margin_mm: number } };
  expect(request.weed.margin_mm).toBe(50);
});

// The head does not travel to a pass that will not be cut, so switching one off is a travel
// edit too — a preview that kept routing through it would draw motion the machine won't make.
test("disabling a pass replans travel without it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.getByTestId("cut-pass-row").first().getByRole("checkbox").uncheck();

  const requests = await page.evaluate(
    () => (window as unknown as { __travelRequests?: { key: string; enabled: boolean }[][] }).__travelRequests,
  );
  // Both passes still named — the disabled one is dropped from the travel by the planner,
  // not from the list, so a pass going missing stays distinguishable from a frontend bug.
  expect(requests).toEqual([[
    { key: "color:ff0000ff", enabled: false },
    { key: "color:00ff00ff", enabled: true },
  ]]);
});

test("reordering after a doc edit surfaces the stale plan instead of stale travel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  // Same off-screen edit as the stale-cut test above: the dialog covers the canvas.
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
      "commit_transform",
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
    ),
  );

  await reorderPass(page, 0, 1);
  await expect(page.getByText("Your design changed.")).toBeVisible();
});

// The order these two settle in is the whole defect: a reorder issued before Replan carries
// the old revision, so it is refused — and that refusal arriving *after* the fresh plan
// installed used to re-raise the banner the replan had just cleared, telling the operator a
// document they had only now replanned was stale again.
test("a reorder refused for the old revision does not re-mark a freshly replanned document", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
      "commit_transform",
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
    ),
  );
  const banner = page.getByText("Your design changed.");
  await startCut(page);
  await expect(banner).toBeVisible();
  await goToSendStep(page, 2);

  // The interleaving, without editing a row during a replan — which the dialog now refuses,
  // because rows belonging to a plan being replaced must not accept edits the arriving plan
  // discards. The reorder goes out *first* and is held, which is what the defect's own story
  // says anyway: "a reorder issued before Replan carries the old revision".
  await page.evaluate(() => (window as unknown as { __armHold: () => void }).__armHold());
  await reorderPass(page, 0, 1);
  await page.getByRole("button", { name: "Replan" }).click();

  await page.evaluate(() => (window as unknown as { __releasePlans: () => Promise<unknown> }).__releasePlans());
  await expect(banner).toHaveCount(0);

  // The late refusal for the revision the fresh plan replaced must not re-raise the banner.
  await page.evaluate(() => (window as unknown as { __releaseTravel: () => Promise<unknown> }).__releaseTravel());
  await expect(banner).toHaveCount(0);
});

// The sheet's own material control, not the per-row picker and not the properties panel
// (those share the accessible name). One choice is every pass, and a typed speed would
// otherwise hide the preset's own.
test("a material preset applies to every pass and clears a typed level", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedUserPreset: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  const dialog = page.getByRole("dialog", { name: "Send to cutter" });
  await fillSendLevels(page);
  await expect(dialog.getByLabel("Speed for pass 1")).toHaveValue("10");

  await dialog.getByLabel("Material preset").selectOption("preset:cameo5-htv");
  await expect(dialog.getByLabel("Preset for pass 1")).toHaveValue("preset:cameo5-htv");
  await expect(dialog.getByLabel("Preset for pass 2")).toHaveValue("preset:cameo5-htv");
  // The typed 10 is gone; the field shows the preset's own speed.
  await expect(dialog.getByLabel("Speed for pass 1")).toHaveValue("5");
  await expect(dialog.getByLabel("Force for pass 1")).toHaveValue("20");

  await dialog.getByLabel("Preset for pass 2").selectOption("no-preset");
  await expect(dialog.getByLabel("Material preset")).toHaveValue("mixed");
  await dialog.getByLabel("Material preset").evaluate((el) => {
    el.dispatchEvent(new Event("change", { bubbles: true }));
  });
  await expect(dialog.getByLabel("Preset for pass 1")).toHaveValue("preset:cameo5-htv");
  await expect(dialog.getByLabel("Preset for pass 2")).toHaveValue("no-preset");

  await dialog.getByLabel("Material preset").selectOption("preset:card-stock");
  await showPresetEditor(page);
  await page.getByLabel("Preset to manage").selectOption("card-stock");
  await page.getByLabel("Delete preset").click();
  // The id is no longer in the list. The select keeps it, rather than falling through to No preset.
  await expect(dialog.getByLabel("Material preset")).toHaveValue("preset:card-stock");
});

// The one test that drives the Cut Host surface end to end. It is here rather than in a unit
// test because the parts it checks are exactly the ones a unit test cannot: that the grouped
// list reaches the dialog at all, and that a refusal from the backend leaves the row on screen.
test("an unreachable host keeps its cutters listed, and refusing to be forgotten keeps its row", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedBusyHost: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();

  // Listed with the reason, not hidden — a cutter that vanishes looks exactly like one that was
  // never paired (#42). The name is also the card's connection line, so the first match is enough.
  await expect(page.getByText("Workshop Pi").first()).toBeVisible();
  await expect(page.getByText("the host could not be reached (timed out)")).toBeVisible();
  // Listed, and not choosable: the card is the connect action, and it stays off until the host answers.
  const hostCutters = page.getByRole("radiogroup", { name: "Workshop Pi" });
  await expect(hostCutters.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true })).toBeDisabled();
  // Its cutter is still a row: two local cutters plus this one. None has been polled, so every
  // badge says so rather than offering a cut for a status nobody has asked for.
  await expect(page.getByText("Unknown")).toHaveCount(3);
  // Nothing has been refused yet, so the warning is nowhere — including over the two local
  // cutters, whose section has no host id at all for a refusal to match (#265).
  const forceWarning = page.getByText(/Forgetting this Cut Host discards the credentials/);
  await expect(forceWarning).toHaveCount(0);

  await page.getByRole("button", { name: "Forget Workshop Pi" }).click();
  // The Rust side's own words, and the row still there (#94).
  await expect(page.getByText("this Cut Host could not be asked whether it is cutting")).toBeVisible();
  await expect(page.getByText("Workshop Pi").first()).toBeVisible();

  // Only now is the force on screen, and it says what is being accepted rather than asking
  // whether the operator is sure. Once, against the host that refused: the local section is still
  // listed below it and must not have grown a copy.
  await expect(forceWarning).toHaveCount(1);
  await page.getByRole("button", { name: "Discard Workshop Pi anyway" }).click();
  // A Pi that is gone for good must not become unforgettable — the row and its cutter both go.
  await expect(page.getByRole("button", { name: /^Forget/ })).toHaveCount(0);
  expect(await page.getByText("Workshop Pi").count()).toBe(0);
});

// The sentence is the Rust side's, shown in the slot a connection error already uses. Both
// directions: the host is the side to update, and this desktop is. The cutters stay listed and
// not choosable, the same as any other host that cannot be asked.
test("a protocol mismatch says which side to update", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedProtocolMismatch: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();

  const hostOlder = "This Cut Host is running an older version. Update your Cut Host to connect.";
  const desktopOlder = "This version of Cuthulhu is older than the Cut Host. Update Cuthulhu to connect.";
  // Not `{ exact: true }`: the sentence and the "How to update" link share this
  // element, so the element's text is longer than the sentence.
  await expect(page.getByText(hostOlder)).toBeVisible();
  await expect(page.getByText(desktopOlder)).toBeVisible();
  await expect(page.getByRole("radiogroup", { name: "Workshop Pi", exact: true }).getByRole("radio")).toBeDisabled();
  await expect(page.getByRole("radiogroup", { name: "Office Pi", exact: true }).getByRole("radio")).toBeDisabled();
  // The chip and the sentence name the same fact. A timed-out host stays "Unknown".
  await expect(page.getByRole("radiogroup", { name: "Workshop Pi", exact: true }).getByTestId("device-badge")).toHaveText("Update needed");
  await expect(page.getByRole("radiogroup", { name: "Office Pi", exact: true }).getByTestId("device-badge")).toHaveText("Update needed");
  const how = page.getByRole("link", { name: "How to update" });
  await expect(how).toHaveCount(2);
  await expect(how.nth(0)).toHaveAttribute("href", "https://github.com/vcolombo/cuthulhu/blob/main/docs/cut-host.md#update");
  await expect(how.nth(1)).toHaveAttribute("href", "https://github.com/vcolombo/cuthulhu/releases");
});

// The list poll also calls `list_hosts` once a second, so a Retry test that merely waits for
// the counter to move stays green when the button does nothing. Time is frozen here, and the
// click is the only thing that can ask again.
test("Retry on an unreachable host asks for the list again", async ({ page }) => {
  // `install()` alone leaves the fake clock running, and the sheet polls `list_hosts` every
  // second while a host is paired. `expect.poll` waits five seconds, so that tick alone reaches
  // `listed + 1` and a Retry that does nothing still passes. `pauseAt` is what stops the tick.
  // The pause is after the row is on screen: pausing through load stalls the timers the page
  // needs to finish opening. Jumping to the pause fires a timer that was already due at most
  // once, so the count is read after that jump, not before it.
  await page.clock.install();
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedBusyHost: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await expect(page.getByRole("button", { name: "Retry Workshop Pi" })).toBeVisible();
  const pageNow = await page.evaluate(() => Date.now());
  await page.clock.pauseAt(pageNow + 1000);
  const listed = await page.evaluate(() => (window as unknown as { __hostListCalls?: number }).__hostListCalls ?? 0);
  await page.getByRole("button", { name: "Retry Workshop Pi" }).click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __hostListCalls?: number }).__hostListCalls ?? 0)).toBe(listed + 1);
  // Wall time, with the page clock still paused. A tick that was only quiet for the click
  // would land inside this wait and the count would move.
  await page.waitForTimeout(1500);
  expect(await page.evaluate(() => (window as unknown as { __hostListCalls?: number }).__hostListCalls ?? 0)).toBe(listed + 1);
});

// The absence is its own test because every path to the warning goes through a host, and a desktop
// with none paired has no Forget to press: the defect it covers (#265) needed no interaction at all,
// so a test that starts by pairing something can never see it.
test("a desktop with no Cut Host paired shows no force-forget warning", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();

  // The local cutters are listed — the dialog is populated, so the warning's absence is about the
  // guard rather than about an empty device section.
  await expect(page.getByTestId("device-badge")).toHaveCount(2);
  await expect(page.getByText(/Forgetting this Cut Host discards the credentials/)).toHaveCount(0);
  await expect(page.getByRole("button", { name: /anyway$/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /^Keep / })).toHaveCount(0);
});

// Reaching a Pi starts here, and nothing drove it end to end while the fake refused the three
// pairing commands. The order is the security property: the fingerprint reaches the operator
// before the token reaches the host, so a rejected fingerprint has told the far end nothing.
test("pairing a Cut Host shows its fingerprint first, then lists it with its cutters", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  const addHost = page.getByRole("button", { name: /Add a Cut Host/ });
  await addHost.click();
  const pair = page.getByRole("dialog", { name: "Pair a Cut Host" });
  await pair.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(pair).toHaveCount(0);
  await expect(addHost).toBeFocused();
  await addHost.click();

  await page.getByLabel("Address").fill("pi.local:7878");
  await page.getByLabel("Token").fill("correct-horse");
  await page.getByLabel("Name (optional)").fill("Workshop Pi");
  await page.getByRole("button", { name: "Pair", exact: true }).click();

  await expect(page.getByText("AB:CD:EF:01:23:45")).toBeVisible();
  await page.getByRole("button", { name: /It matches/ }).click();

  // The host is in the device list with the cutter the Test proved, and it can be forgotten —
  // the whole affordance, from an empty list to a usable remote cutter.
  await expect(page.getByRole("button", { name: "Forget Workshop Pi" })).toBeVisible();
  await expect(page.getByText("pi.local:7878").first()).toBeVisible();
});

// A wrong token must not leave a host saved. `pair_host` is what persists, and it is never
// reached: the row would otherwise have to be forgotten by hand before a retry could work.
test("a refused token pairs nothing and says so in the host's own words", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("button", { name: /Add a Cut Host/ }).click();

  await page.getByLabel("Address").fill("pi.local:7878");
  await page.getByLabel("Token").fill("wrong");
  await page.getByRole("button", { name: "Pair", exact: true }).click();
  await page.getByRole("button", { name: /It matches/ }).click();

  await expect(page.getByRole("alert")).toHaveText("the token was refused");
  await expect(page.getByRole("button", { name: /^Forget/ })).toHaveCount(0);
});

// A host that is forgotten has to leave with its cutters. Dropping the row alone leaves them
// naming a host nobody is paired with, which is the one thing `groupDevices` refuses to hide:
// the row comes back as a raw host id under "this Cut Host is not paired with this computer".
// The count is taken once, not awaited — the wrong row is the state right after the click, and
// an assertion that retries would sit there until something else repaired it.
test("forgetting a host takes its cutters with it, instead of renaming the row to a raw id", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("button", { name: /Add a Cut Host/ }).click();
  await page.getByLabel("Address").fill("pi.local:7878");
  await page.getByLabel("Token").fill("correct-horse");
  await page.getByLabel("Name (optional)").fill("Workshop Pi");
  await page.getByRole("button", { name: "Pair", exact: true }).click();
  await page.getByRole("button", { name: /It matches/ }).click();
  await expect(page.getByRole("button", { name: "Forget Workshop Pi" })).toBeVisible();

  await page.getByRole("button", { name: "Forget Workshop Pi" }).click();
  await expect(page.getByRole("button", { name: /^Forget/ })).toHaveCount(0);
  expect(await page.getByText("this Cut Host is not paired with this computer").count()).toBe(0);
  expect(await page.getByText("host-2").count()).toBe(0);
});

// The recovery path had both halves tested and the join between them untested: `connectedControl`
// is unit-tested for which control to show, `Host::reconnect` is tested against a real loopback
// Cut Host, and the line that turns the first into the second — `verb === "reconnect" ? ... : ...`
// — was covered by nothing, because the fake's connected device was always local (#123).
//
// A remote fixture rather than a `reconnect_device` handler bolted onto a local one: a fake that
// can ship a wrong command name green is the shape of #85, and the honest version of this test
// needs a remote connected cutter anyway — nothing in this suite had one.
test("a stuck cutter on a Cut Host offers Reconnect, and reconnecting makes it cuttable again", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedRemoteConnected: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();

  // The cancel nobody could confirm: the badge asks for a person, and Start Cut is withheld.
  // Scoped to the dialog: the pill and the status bar name the same host.
  const dialog = page.getByRole("dialog", { name: "Send to cutter" });
  await expect(dialog.getByText("Bench Pi").first()).toBeVisible();
  await expect(page.getByText(/Cancelled — stop not confirmed/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Start Cut" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Next", exact: true })).toBeDisabled();

  // Reconnect, not Disconnect: this desktop never opened that transport, and dropping the aim
  // would leave the cutter exactly as stuck.
  await expect(page.getByRole("button", { name: "Disconnect usb:pi:C" })).toHaveCount(0);
  await page.getByRole("button", { name: "Reconnect usb:pi:C" }).click();

  await expect(page.getByText("Ready", { exact: true })).toBeVisible();
  await expectStartOffered(page);
});

// The teardown is the deliverable, not the interval. A leaked interval keeps a Cut Host
// connection warm forever, and the daemon caps concurrent clients at eight (#103) — a desktop
// that leaks one per dialog-open exhausts a Pi, which then refuses every new connection until
// it is restarted.
const pollCount = (page: import("@playwright/test").Page) => () =>
  page.evaluate(
    () =>
      (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
        "__test_poll_count",
      ) as Promise<number>,
  );

const listCount = (page: import("@playwright/test").Page) => () =>
  page.evaluate(
    () =>
      (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
        "__test_list_count",
      ) as Promise<number>,
  );

test("the dialog polls while it is open, and stops once it is closed", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedBusyHost: true });
  await page.goto("/");
  const polls = listCount(page);

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  const opened = await polls();
  await expect.poll(polls, { timeout: 5000 }).toBeGreaterThan(opened);

  await page.getByRole("button", { name: "Close" }).click();
  // A tick already in flight when the dialog closed still lands, so settle past one full period
  // before taking the reading that must not move. Idle is ten seconds, and closing does not
  // start a tick of its own, so this window stays still.
  await page.waitForTimeout(1500);
  const closed = await polls();
  await page.waitForTimeout(2500);
  expect(await polls()).toBe(closed);
});

// The stale path, on the desktop that has no host: the local section's heading is normally
// suppressed, and the "last known" marker lives inside it, so a failed read showed as one
// unlabelled red line of Rust prose under "Device" — no heading, no marker, no banner.
test("a device list that cannot be read keeps its heading, so the failure has something to name", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, failList: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();

  await expect(page.getByText("This computer")).toBeVisible();
  await expect(page.getByText("last known")).toBeVisible();
  await expect(page.getByText("the device list could not be read")).toBeVisible();
});

// The Pi is optional, and a desktop without one has nothing polling can tell it: a local
// cutter's status is pushed. Before this branch `list_devices` ran once per dialog open, and it
// walks the USB bus and the serial ports — running it every second for a user who owns no host
// is a cost with no answer on the other end of it.
test("a desktop with no Cut Host paired does not poll at all", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.waitForTimeout(2500);
  expect(await pollCount(page)()).toBe(0);
});

// The other half of polling safely: an unreachable host can take seconds to answer, and a tick
// that waits its turn instead of being skipped builds a backlog that outlives whatever wedged it.
test("a tick whose last request is still in flight is skipped, not queued", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedBusyHost: true, slowList: true });
  await page.goto("/");
  const polls = listCount(page);

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  const opened = await polls();
  await page.waitForTimeout(4500);
  // Four ticks have fired; at three seconds an answer, at most two of them can have started
  // requests. Without the skip the count tracks the tick rate instead, and every extra request
  // is one more thing the host owes an answer to. The mount read is already in `opened`.
  const seen = (await polls()) - opened;
  expect(seen).toBeGreaterThanOrEqual(1);
  expect(seen).toBeLessThanOrEqual(2);
});

function devicesShots(): boolean {
  return (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.DEVICES_SHOTS === "1";
}

/** Retry and Forget stay on the title row. The line under them uses the card. */
async function hostCopySpansTheCard(page: Page, heading: string, copy: Locator) {
  const card = page.locator("section").filter({ has: page.getByRole("heading", { name: heading, exact: true }) });
  const cardBox = await card.boundingBox();
  const copyBox = await copy.boundingBox();
  const button = card.getByRole("button", { name: /^Forget / });
  const buttonBox = await button.boundingBox();
  expect(cardBox).not.toBeNull();
  expect(copyBox).not.toBeNull();
  expect(buttonBox).not.toBeNull();
  expect(copyBox!.width).toBeGreaterThan(cardBox!.width * 0.75);
  expect(copyBox!.y).toBeGreaterThanOrEqual(buttonBox!.y + buttonBox!.height - 1);
  expect(buttonBox!.x + buttonBox!.width).toBeGreaterThan(cardBox!.x + cardBox!.width - 28);
  const titleBox = await card.getByRole("heading", { name: heading, exact: true }).boundingBox();
  expect(titleBox).not.toBeNull();
  const sharesRow = buttonBox!.y < titleBox!.y + titleBox!.height && titleBox!.y < buttonBox!.y + buttonBox!.height;
  // A title beside the buttons stays one line. One that cannot fit takes the row.
  if (sharesRow) expect(titleBox!.height).toBeLessThan(28);
  else expect(titleBox!.width).toBeGreaterThan(cardBox!.width * 0.7);
}

test("two cutters on one host keep their own status, and cancel names that cutter", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedTwoCutters: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("tab", { name: "Devices" }).click();

  const cameo = page.getByRole("listitem").filter({ hasText: "Silhouette Cameo 5 Alpha" }).filter({ hasText: "Garage Pi" });
  const puma = page.getByRole("listitem").filter({ hasText: "GCC Puma IV" }).filter({ hasText: "Garage Pi" });
  await expect(cameo.getByText("Cutting", { exact: true })).toBeVisible();
  await expect(puma.getByText("Ready", { exact: true })).toBeVisible();
  await expect(page.getByText("Reconnecting. The cut keeps going on the host.")).toHaveCount(0);
  await expect(cameo.getByText("40% of this pass")).toBeVisible();
  await expect(page.getByText("Add to queue")).toHaveCount(0);
  await expect(page.getByText("Up next")).toHaveCount(0);

  await cameo.getByRole("button", { name: "Cancel", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Cancel the cut", exact: true }).click();
  await expect.poll(() => page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_cancel_targets"),
  )).toEqual([{ host: "host-4", instanceId: "usb:garage:cameo" }]);
  await expect(cameo.getByText("Ready", { exact: true })).toBeVisible();
  await expect(cameo.getByText("Last cut cancelled")).toBeVisible();
  await expect(puma.getByText("Ready", { exact: true })).toBeVisible();
  await expect(puma.getByText("Last cut cancelled")).toHaveCount(0);
  await expect(puma.getByRole("button", { name: /Reconnect/ })).toHaveCount(0);
  await expect(page.getByRole("listitem").filter({ hasText: "USB on this computer" }).getByText("Not checked").first()).toBeVisible();
  await expect(page.getByTestId("devices-panel").getByText("Unknown", { exact: true })).toHaveCount(0);
  for (const list of await page.getByTestId("devices-panel").getByRole("list").all()) {
    expect(await list.evaluate((el) => getComputedStyle(el).listStyleType)).toBe("none");
  }
  if (devicesShots()) await page.screenshot({ path: "/opt/cursor/artifacts/devices-two-cutters-light.png" });
  if (devicesShots()) {
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: "Dark", exact: true }).click();
    await page.screenshot({ path: "/opt/cursor/artifacts/devices-two-cutters-dark.png" });
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: "Light", exact: true }).click();
  }
});

test("an offline host keeps the cut frozen, then shows the reading from after it reconnects", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedTwoCutters: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("tab", { name: "Devices" }).click();
  await expect(page.getByText("40% of this pass")).toBeVisible();

  await callFakeWith(page, "__test_host_unreachable", { id: "host-4", unreachable: "the host could not be reached (timed out)" });
  await expect(page.getByRole("heading", { name: "Can't reach Garage Pi" })).toBeVisible();
  // After the card is up, the next reading is the one that will not stamp over this.
  await callFakeWith(page, "__test_set_last_seen", { id: "host-4", at: Date.now() - 2 * 60 * 60 * 1000 });
  await expect(page.getByText("Last seen 2 hours ago · garage.local:7878")).toBeVisible();
  await expect(page.getByText("the host could not be reached (timed out)")).toBeHidden();
  await page.getByTestId("devices-panel").locator("details").getByText("Details").click();
  await expect(page.getByText("the host could not be reached (timed out)")).toBeVisible();
  await page.getByTestId("devices-panel").locator("details").getByText("Details").click();
  await expect(page.getByText("the host could not be reached (timed out)")).toBeHidden();
  await expect(page.getByText(/Connection lost · last update/)).toHaveCount(0);
  await expect(page.getByText("Reconnecting. The cut keeps going on the host.")).toBeVisible();
  await expect(page.getByTestId("pass-bar").locator(".progress-live")).toHaveCount(0);
  await expect(page.getByText("40% of this pass")).toBeVisible();
  // This desktop is not tracking the seeded cut, so Cancel is not a queue.
  const offlineCameo = page.getByRole("listitem").filter({ hasText: "Silhouette Cameo 5 Alpha" }).filter({ hasText: "Garage Pi" });
  await expect(page.getByRole("button", { name: "Cancel (when reconnected)", exact: true })).toHaveCount(0);
  await expect(offlineCameo.getByRole("button", { name: "Cancel", exact: true })).toBeDisabled();
  await expect(page.getByRole("listitem").filter({ hasText: "GCC Puma IV" }).filter({ hasText: "Garage Pi" }).getByText("Unavailable")).toBeVisible();

  const body = page.getByRole("heading", { name: "Can't reach Garage Pi" });
  const muted = page.getByTestId("devices-muted");
  const warning = page.getByTestId("devices-warning");
  const lightBody = await paintContrast(page, body);
  const lightMuted = await paintContrast(page, muted);
  const lightWarning = await paintContrast(page, warning);
  expect(lightBody.ratio).toBeGreaterThanOrEqual(4.5);
  expect(lightMuted.ratio).toBeGreaterThanOrEqual(4.5);
  expect(lightWarning.ratio).toBeGreaterThanOrEqual(4.5);
  await hostCopySpansTheCard(page, "Can't reach Garage Pi", warning);
  if (devicesShots()) await page.screenshot({ path: "/opt/cursor/artifacts/devices-offline-light.png" });

  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Dark", exact: true }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  const darkBody = await paintContrast(page, body);
  const darkMuted = await paintContrast(page, muted);
  const darkWarning = await paintContrast(page, warning);
  expect(darkBody.ratio).toBeGreaterThanOrEqual(4.5);
  expect(darkMuted.ratio).toBeGreaterThanOrEqual(4.5);
  expect(darkWarning.ratio).toBeGreaterThanOrEqual(4.5);
  await hostCopySpansTheCard(page, "Can't reach Garage Pi", warning);
  if (devicesShots()) await page.screenshot({ path: "/opt/cursor/artifacts/devices-offline-dark.png" });
  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Light", exact: true }).click();

  await callFakeWith(page, "__test_set_remote_sent", { host: "host-4", instanceId: "usb:garage:cameo", sent: 90 });
  await callFakeWith(page, "__test_host_unreachable", { id: "host-4", unreachable: null });
  await expect(page.getByText("90% of this pass")).toBeVisible();
  await expect(page.getByTestId("pass-bar").locator(".progress-live")).toHaveCount(1);
  await expect(page.getByRole("heading", { name: "Can't reach Garage Pi" })).toHaveCount(0);
});

test("a protocol mismatch on the devices host card says which side to update", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedProtocolMismatch: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("tab", { name: "Devices" }).click();

  const hostOlder = "This Cut Host is running an older version. Update your Cut Host to connect.";
  const desktopOlder = "This version of Cuthulhu is older than the Cut Host. Update Cuthulhu to connect.";
  await expect(page.getByRole("heading", { name: "Workshop Pi needs an update" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Update Cuthulhu to use Office Pi" })).toBeVisible();
  await expect(page.getByText("Can't reach", { exact: false })).toHaveCount(0);
  await expect(page.getByText("No successful reading yet")).toHaveCount(0);
  await expect(page.getByText(hostOlder)).toBeVisible();
  await expect(page.getByText(desktopOlder)).toBeVisible();
  await expect(page.getByText("Update needed")).toHaveCount(2);
  const workshop = page.getByRole("listitem").filter({ hasText: "Workshop Pi" });
  const office = page.getByRole("listitem").filter({ hasText: "Office Pi" });
  await expect(workshop.getByRole("button", { name: "Set as active" })).toBeDisabled();
  await expect(workshop.getByRole("button", { name: "Set as active" })).toHaveAccessibleDescription("Workshop Pi needs an update");
  await expect(office.getByRole("button", { name: "Set as active" })).toBeDisabled();
  await expect(office.getByRole("button", { name: "Set as active" })).toHaveAccessibleDescription("Update Cuthulhu to use Office Pi");
  const how = page.getByRole("link", { name: "How to update" });
  await expect(how).toHaveCount(2);
  await expect(how.nth(0)).toHaveAttribute("href", "https://github.com/vcolombo/cuthulhu/blob/main/docs/cut-host.md#update");
  await expect(how.nth(1)).toHaveAttribute("href", "https://github.com/vcolombo/cuthulhu/releases");
  await page.getByRole("button", { name: "Add cutter" }).focus();
  await expect(page.getByRole("button", { name: "Add cutter" })).toBeFocused();
  await expect.poll(() => page.getByRole("button", { name: "Add cutter" }).evaluate((el) => getComputedStyle(el).boxShadow)).not.toBe("none");
  const active = page.getByRole("button", { name: "Set as active" }).first();
  await active.focus();
  await expect(active).toBeFocused();
  await hostCopySpansTheCard(page, "Workshop Pi needs an update", page.getByText(hostOlder));
  await hostCopySpansTheCard(page, "Update Cuthulhu to use Office Pi", page.getByText(desktopOlder));
  if (devicesShots()) await page.screenshot({ path: "/opt/cursor/artifacts/devices-mismatch-light.png" });
  if (devicesShots()) {
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: "Dark", exact: true }).click();
    await hostCopySpansTheCard(page, "Workshop Pi needs an update", page.getByText(hostOlder));
    await hostCopySpansTheCard(page, "Update Cuthulhu to use Office Pi", page.getByText(desktopOlder));
    await page.screenshot({ path: "/opt/cursor/artifacts/devices-mismatch-dark.png" });
  }
});

test("cancel mid-cut shows Cancelled and re-enables Start Cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();

  await page.getByRole("button", { name: "Cancel job", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Cancel the cut", exact: true }).click();
  await expect(page.getByRole("region", { name: "Job" }).getByText("Cancelled", { exact: true })).toBeVisible();
  await expectStartOffered(page);
});

test("a cut that runs to the end reports completion, not a cancellation", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();

  // The backend says which ending it was, so the two endings cannot be confused: this
  // is the case the mock could not express while a finished cut only rested on `Idle`.
  await expect(page.getByRole("region", { name: "Job" }).getByText("Completed", { exact: true })).toBeVisible();
  await expect(page.getByText("Cancelled", { exact: true })).toHaveCount(0);
  await expectStartOffered(page);
});

test("transmitting shows a Cancel button and progress so the GUI can cancel mid-cut", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  // Sending lasts 50ms unless held, and the badge is on step 1. Without the hold the
  // pass has already parked for a colour swap, whose badge asks for a person.
  await page.evaluate(() => (window as unknown as { __holdPass: () => void }).__holdPass());
  await startCut(page);
  const job = page.getByRole("region", { name: "Job" });
  await expect(job.getByTestId("pass-progress")).toHaveText("0% of this pass");
  await expect(job.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();

  // The tone reaches the row, so the cutting one is not the same flat grey as the cutter nobody
  // has polled. `deviceBadge`'s unit tests assert on `tone` alone; this is what makes them
  // describe the product instead of a field it threw away. The unpolled row is `unknown`, not
  // `attention` — nothing is wrong with it, and red is how this UI says something is.
  await expect(page.getByTestId("device-badge").first()).toHaveAttribute("data-tone", "busy");
  await expect(page.getByTestId("device-badge").first()).toHaveText("Cutting");
  await expect(page.getByTestId("device-badge").nth(1)).toHaveAttribute("data-tone", "unknown");
});

test("a second cut replaces a finished job before its Sending event", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedRemoteIdle: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radiogroup", { name: "Bench Pi" }).getByRole("radio", { name: "Silhouette Cameo 5 Alpha" }).click();
  await showPasses(page);
  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByRole("region", { name: "Job" }).getByText("Completed", { exact: true })).toBeVisible();

  await page.evaluate(() => (window as unknown as { __holdCutEvents: () => void }).__holdCutEvents());
  await startCut(page);
  const job = page.getByRole("region", { name: "Job" });
  await expect(job.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();
  await expect(page.getByText("Completed", { exact: true })).toHaveCount(0);
  // The host poll is 1s, and the first read is still the previous ending.
  await page.waitForTimeout(1500);
  await expect(job.getByRole("button", { name: "Cancel", exact: true })).toBeVisible();
  await expect(page.getByText("Completed", { exact: true })).toHaveCount(0);

  await page.evaluate(() => (window as unknown as { __releaseCutEvents: () => void }).__releaseCutEvents());
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume" })).toBeVisible();
});

test("second cut in the same dialog session also reaches waiting-for-swap", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByText("Completed", { exact: true })).toBeVisible();

  // Second cut in the same session: the sheet opens on step 1 again, and the new job's
  // statuses replace the finished one. Nothing is remembered per job id.
  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume" })).toBeVisible();
});

test("reopening the dialog after connect recovers the connected device", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  const cut = page.getByRole("dialog", { name: "Send to cutter" });
  const cameo = cut.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first();
  await expect(cameo).toHaveAttribute("aria-checked", "true");
  await expect(cut.getByTestId("device-badge").first()).toHaveText("Ready");

  await page.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();

  // Without get_connected_device seeding this on mount, the reopened dialog's local
  // `connected` state comes back null even though the backend is still connected,
  // leaving Start Cut stuck disabled and the card unchecked.
  const reopened = page.getByRole("dialog", { name: "Send to cutter" });
  await expect(reopened.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first()).toHaveAttribute("aria-checked", "true");
  await expect(reopened.getByTestId("device-badge").first()).toHaveText("Ready");
  await expectStartOffered(page);
});

// The exit from a cancel whose stop nothing confirmed. `driver-core` refuses both a cut and a
// connect from that state, so with no Disconnect in the dialog the operator's only way back to
// their own cutter is restarting the app — the workaround being "never cancel". This drives the
// control rather than the command behind it, because the control is what was missing.
test("a connected cutter offers a disconnect, and choosing it clears the aim", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  const cut = page.getByRole("dialog", { name: "Send to cutter" });
  const cameo = cut.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first();
  await expect(cameo).toHaveAttribute("aria-checked", "true");
  await expect(cut.getByTestId("device-badge").first()).toHaveText("Ready");

  await page.getByRole("button", { name: "Disconnect", exact: false }).first().click();
  await expect(cameo).toHaveAttribute("aria-checked", "false");
  await expect(cut.getByRole("button", { name: /^Disconnect/ })).toHaveCount(0);
});

// The ending lives on the job panel, which is what the device last did. Reopening the
// sheet must not repeat it and must not clear it. The guard against a false completion
// is "disconnecting mid-pause" below: there the job never ended, and no banner appears.
test("a reopened sheet leaves the ending on the job panel", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();
  const job = page.getByRole("region", { name: "Job" });
  await expect(job.getByText("Completed", { exact: true })).toBeVisible();

  // The ending stays on the panel. Reopening the sheet must not repeat it, and must not clear it.
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Send to cutter" });
  await expect(dialog.getByText("Completed", { exact: true })).toHaveCount(0);
  await expect(dialog.getByText("Job complete")).toHaveCount(0);
  await page.getByRole("button", { name: "Close" }).click();
  await expect(job.getByText("Completed", { exact: true })).toBeVisible();
  await expect(page.getByText("Cancelled", { exact: true })).toHaveCount(0);
});

test("failed cut shows Cut failed and a reconnect recovers the device", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();

  // Arm a one-shot async resume failure, then resume: the Failed phase must show
  // "Cut failed" — never a completion, which a banner derived from "a job ended
  // and the device is Idle" produced for a failed job whose state cache lagged.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_resume"));
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByRole("region", { name: "Job" }).getByText("Cut failed")).toBeVisible();
  await expect(page.getByText("Completed", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Job complete")).toHaveCount(0);

  // Recover by reconnecting (the other listed device). The connect lifecycle events
  // carry NO_JOB=0 and must still reach the dialog after a failed job — Start Cut
  // comes back only because the reconnect's Idle status was accepted.
  // The first choice aimed the document at the Cameo, so this one asks before switching. The
  // answer is the fake dialog plugin's, scripted here: the plugin raises no native dialog.
  // The aimed card is not a second connect: the Puma card is the other cutter.
  await page.evaluate(() => sessionStorage.setItem("__dialog_answer__", "ok"));
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "GCC Puma IV", exact: true }).click();
  await expectStartOffered(page);
});

// A fault whose mid-flight status never reaches a render: a banner that waits to be
// handed a "a cut was running" status first shows nothing at all here.
test("a cut that faults immediately still shows Cut failed", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("__test_fail_next_cut"));
  await startCut(page);
  await expect(page.getByRole("region", { name: "Job" }).getByText("Cut failed")).toBeVisible();
  await expect(page.getByText("Completed", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Job complete")).toHaveCount(0);
});

// A completed banner belongs to the connection that cut it: a reconnect is a fresh
// device that has cut nothing, so the ending must go with the old connection. This is
// the reconnect half of what the dialog's latch used to get for free by remounting —
// the status has to clear it, and driver-core's lifecycle emit is what does.
test("a reconnect clears the completed banner from the previous connection", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
  await page.getByRole("button", { name: "Resume" }).click();
  await expect(page.getByText("Completed", { exact: true })).toBeVisible();

  // Driven through the IPC surface rather than the dialog's own Disconnect button: this is
  // about the device dropping out, not about the operator asking it to.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("disconnect_device"));
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await expectStartOffered(page);
  await expect(page.getByText("Completed", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Job complete")).toHaveCount(0);
});

// Losing the device mid-pause abandons the job. The reconnect reports Idle — the same
// phase a finished cut rests on — so a dialog that remembers "a cut was running" across
// the disconnect declares a job that never finished complete.
test("disconnecting a remote cutter mid-pause removes the swap", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedRemoteIdle: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radiogroup", { name: "Bench Pi" }).getByRole("radio", { name: "Silhouette Cameo 5 Alpha" }).click();
  await showPasses(page);
  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();

  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("disconnect_device"),
  );
  await expect(page.getByRole("button", { name: "Resume" })).toHaveCount(0);
  await expect(page.getByText(/complete/i)).toHaveCount(0);
});

test("a cancel queued during an outage is dropped when the cut has already finished", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedRemoteIdle: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radiogroup", { name: "Bench Pi" }).getByRole("radio", { name: "Silhouette Cameo 5 Alpha" }).click();
  await showPasses(page);
  await startCut(page);
  const swap = page.getByRole("region", { name: "Colour swap" });
  await expect(swap).toBeVisible();
  await callFakeWith(page, "__test_host_unreachable", { id: "host-3", unreachable: "the host could not be reached (timed out)" });
  await expect(swap.getByRole("button", { name: "Cancel (when reconnected)", exact: true })).toBeVisible();
  await swap.getByRole("button", { name: "Cancel (when reconnected)", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Cancel the cut", exact: true }).click();
  await callFakeWith(page, "__test_set_status", {
    phase: "Idle",
    actions: { cut: true },
    rest: { ended: "Completed" },
  });
  await callFakeWith(page, "__test_host_unreachable", { id: "host-3", unreachable: null });
  const job = page.getByRole("region", { name: "Job" });
  await expect(job.getByText("Completed", { exact: true })).toBeVisible();
  await expect(page.getByText("Cancelled", { exact: true })).toHaveCount(0);
});

test("disconnecting mid-pause does not report the abandoned cut as complete", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await showPasses(page);
  await expect(page.getByTestId("cut-pass-row")).toHaveCount(2);

  await startCut(page);
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();

  // Driven through the IPC surface rather than the dialog's own Disconnect button: this is
  // about the device dropping out, not about the operator asking it to.
  await page.evaluate(() => (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("disconnect_device"));
  await expect(page.getByRole("button", { name: "Resume" })).toHaveCount(0);

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await expectStartOffered(page);
  await expect(page.getByText(/complete/i)).toHaveCount(0);
});

test("an unreachable host freezes the pass, then shows the real status", async ({ page }) => {
  test.setTimeout(60_000);
  await page.addInitScript(installMockTauri, { seedRedGold: true, seedRemoteIdle: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radiogroup", { name: "Bench Pi" }).getByRole("radio", { name: "Silhouette Cameo 5 Alpha" }).click();
  await page.evaluate(() => (window as unknown as { __holdPass: () => void }).__holdPass());
  await startCut(page);

  const job = page.getByRole("region", { name: "Job" });
  await expect(job.getByText("Red vinyl")).toBeVisible();
  await expect(job.getByTestId("pass-progress")).toHaveText("0% of this pass");
  await callFakeWith(page, "__test_set_progress", { sent: 40 });
  await expect(job.getByTestId("pass-progress")).toHaveText("40% of this pass");

  await callFakeWith(page, "__test_host_unreachable", { id: "host-3", unreachable: "the host could not be reached (timed out)" });
  await expect(job.getByText(/Connection lost · last update \d+s ago/)).toBeVisible();
  await callFakeWith(page, "__test_set_progress", { sent: 90 });

  const ageOf = async () => {
    const text = await job.getByText(/Connection lost · last update \d+s ago/).innerText();
    return Number(text.match(/(\d+)s ago/)?.[1] ?? "0");
  };
  const before = await ageOf();
  const started = Date.now();
  if ((globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.JOB_PANEL_SHOTS === "1") {
    await page.screenshot({ path: "/opt/cursor/artifacts/job-offline-light.png" });
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: "Dark", exact: true }).click();
    await page.screenshot({ path: "/opt/cursor/artifacts/job-offline-dark.png" });
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: "Light", exact: true }).click();
  }
  const remaining = 5000 - (Date.now() - started);
  if (remaining > 0) await page.waitForTimeout(remaining);

  expect(await ageOf()).toBeGreaterThanOrEqual(before + 3);
  await expect(job.getByTestId("pass-progress")).toHaveText("40% of this pass");
  await expect(job.getByTestId("pass-bar")).toHaveAttribute("aria-valuenow", "40");
  await expect(job.getByTestId("pass-bar").locator(".progress-live")).toHaveCount(0);
  await expect(job.getByRole("button", { name: "Cancel (when reconnected)", exact: true })).toBeVisible();
  await expect(job.getByText("bench.local:7878")).toBeVisible();

  await callFakeWith(page, "__test_host_unreachable", { id: "host-3", unreachable: null });
  await expect(job.getByTestId("pass-progress")).toHaveText("90% of this pass");
  await expect(job.getByTestId("pass-bar").locator(".progress-live")).toHaveCount(1);
});

test("resume is disabled while a colour swap's host is unreachable", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedRemoteIdle: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radiogroup", { name: "Bench Pi" }).getByRole("radio", { name: "Silhouette Cameo 5 Alpha" }).click();
  await startCut(page);
  const swap = page.getByRole("region", { name: "Colour swap" });
  await expect(swap).toBeVisible();
  await callFakeWith(page, "__test_host_unreachable", { id: "host-3", unreachable: "the host could not be reached (timed out)" });
  const resume = swap.getByRole("button", { name: "Resume", exact: true });
  await expect(resume).toBeDisabled();
  await resume.locator("xpath=..").hover();
  await expect(page.getByRole("tooltip")).toHaveText("Waiting for the host to reconnect");
  await swap.getByRole("button", { name: "Cancel (when reconnected)", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Cancel the cut", exact: true }).click();
  await callFakeWith(page, "__test_host_unreachable", { id: "host-3", unreachable: null });
  await expect(page.getByRole("region", { name: "Job" }).getByText("Cancelled", { exact: true })).toBeVisible();
});

function jobShots(): boolean {
  return (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.JOB_PANEL_SHOTS === "1";
}

async function shot(page: Page, name: string) {
  if (!jobShots()) return;
  await page.screenshot({ path: `/opt/cursor/artifacts/job-${name}-light.png` });
  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Dark", exact: true }).click();
  await page.screenshot({ path: `/opt/cursor/artifacts/job-${name}-dark.png` });
  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Light", exact: true }).click();
}

test("the job panel, swap bar, confirm, cancel and failure states", async ({ page }) => {
  test.setTimeout(60_000);
  await page.addInitScript(installMockTauri, { seedRedGold: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await page.evaluate(() => (window as unknown as { __holdPass: () => void }).__holdPass());
  await startCut(page);

  const job = page.getByRole("region", { name: "Job" });
  await expect(job.getByText("Pass 1 of 2")).toBeVisible();
  await expect(job.getByText("Red vinyl")).toBeVisible();
  await expect(page.getByRole("button", { name: "Send to cutter", exact: true })).toBeVisible();
  await expect(page.getByTestId("job-chip")).toBeVisible();
  await callFakeWith(page, "__test_set_progress", { sent: 40 });
  await expect(job.getByTestId("pass-progress")).toHaveText("40% of this pass");
  await expect(job.getByTestId("pass-bar").locator(".progress-live")).toHaveCount(1);
  await shot(page, "panel-mid-pass");

  await page.evaluate(() => (window as unknown as { __releasePass: () => void }).__releasePass());
  const swap = page.getByRole("region", { name: "Colour swap" });
  await expect(swap.getByText("Load gold vinyl, then press Resume")).toBeVisible();
  await expect(swap.getByText(/pass 2 of 2 · 1 shape/)).toBeVisible();
  await expect(page.getByTestId("job-chip")).toBeVisible();
  await shot(page, "swap-waiting");

  await swap.getByRole("button", { name: "Collapse", exact: true }).click();
  await expect(swap).toHaveCount(0);
  const chip = page.getByTestId("swap-chip");
  await expect(chip).toBeVisible();
  await expect(chip).toHaveAttribute("aria-label", "Show colour swap: Load gold vinyl, then press Resume");
  await shot(page, "swap-chip");
  await chip.click();
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();

  await callFakeWith(page, "__test_set_status", {
    phase: "AwaitingConfirmation",
    actions: { cancel: true, confirm: true },
    rest: { pass: { index: 1, total: 2 }, sent: { sent: 100, total: 100 } },
  });
  await expect(job.getByRole("button", { name: "Confirm pass done", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Resume", exact: true })).toHaveCount(0);
  await expect(job.getByTestId("pass-bar").locator(".progress-live")).toHaveCount(0);
  await expect(job.getByText("Gold vinyl")).toBeVisible();
  await shot(page, "confirm-pass");

  await job.getByRole("button", { name: "Cancel", exact: true }).click();
  const alert = page.getByRole("alertdialog");
  await expect(alert.getByRole("heading", { name: "Cancel this cut?" })).toBeVisible();
  if (jobShots()) {
    await page.screenshot({ path: "/opt/cursor/artifacts/job-cancel-confirm-light.png" });
    await alert.getByRole("button", { name: "Keep cutting", exact: true }).click();
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: "Dark", exact: true }).click();
    await job.getByRole("button", { name: "Cancel", exact: true }).click();
    await page.screenshot({ path: "/opt/cursor/artifacts/job-cancel-confirm-dark.png" });
    await page.getByRole("alertdialog").getByRole("button", { name: "Keep cutting", exact: true }).click();
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: "Light", exact: true }).click();
  } else {
    await alert.getByRole("button", { name: "Keep cutting", exact: true }).click();
  }
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  await expect(job.getByRole("button", { name: "Confirm pass done", exact: true })).toBeVisible();

  await callFakeWith(page, "__test_set_status", {
    phase: "Failed",
    rest: { error: "Timeout" },
  });
  await expect(job.getByText("Cut failed")).toBeVisible();
  await expect(job.getByText("The cutter did not answer in time.")).toBeVisible();
  await expect(job.getByRole("button", { name: "Start again", exact: true })).toBeVisible();
  await shot(page, "failed");
  await job.getByRole("button", { name: "Start again", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Send to cutter" })).toBeVisible();
});

test("the send sheet walks machine, passes and preview, and withholds a blank level", async ({ page }) => {
  test.setTimeout(90_000);
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/");

  const shots = (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.SEND_SHEET_SHOTS === "1";
  const shot = async (name: string) => {
    if (!shots) return;
    await page.getByRole("dialog", { name: "Send to cutter" }).screenshot({ path: `/opt/cursor/artifacts/${name}.png` });
  };
  const move = (id: number, dx: number) =>
    page.evaluate(([node, x]) =>
      (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke(
        "commit_transform",
        { ids: [node], m: [1, 0, 0, 1, x, 0] },
      ), [id, dx] as [number, number]);
  // The seeded rects share an origin. Parting the green one is what lets one shot show both passes.
  await move(3, 30);

  const walk = async (theme: "light" | "dark", dx: number) => {
    // The light walk leaves the Cameo aimed, and a reopened sheet shows that. Step 1 of each
    // theme is the list before a choice.
    await page.evaluate(() =>
      (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string) => Promise<unknown> } }).__TAURI_INTERNALS__.invoke("disconnect_device"),
    );
    // A pointer click on Send lands again on the card once the sheet covers that button, and the
    // card is the connect action. Opening from the element itself leaves every cutter unchosen.
    await page.getByRole("button", { name: "Send to cutter", exact: true }).evaluate((el: HTMLElement) => el.click());
    const dialog = page.getByRole("dialog", { name: "Send to cutter" });
    await expect(dialog).toHaveAttribute("data-send-step", "1");
    await expect(dialog.getByRole("button", { name: "Start Cut" })).toHaveCount(0);
    await expect(dialog.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first()).toBeVisible();
    await expect(dialog.getByText("USB on this computer").first()).toBeVisible();
    await expect(dialog.getByTestId("device-badge").first()).toHaveText("Unknown");
    await shot(`send-step1-${theme}`);

    await showPasses(page);
    await expect(dialog.getByTestId("cut-pass-row")).toHaveCount(2);
    const speed = dialog.getByLabel("Speed for pass 1");
    await expect(speed).toHaveAttribute("aria-invalid", "true");
    const describedBy = await speed.getAttribute("aria-describedby");
    expect(describedBy).toBeTruthy();
    await expect(dialog.locator(`[id="${describedBy}"]`)).toHaveText("Use 1–30");
    await expect(speed).toHaveCSS("border-color", theme === "light" ? "rgb(201, 35, 36)" : "rgb(255, 107, 99)");
    await expect(dialog.getByText("Red vinyl").first()).toBeVisible();
    // The name track used to be min-content, which for a nowrap label is the whole string.
    // That row then grew past the header. Restoring the text before the next paint keeps
    // the shot and the later "Red vinyl" lookup on the real name.
    const nameTrack = await dialog.evaluate((root) => {
      const name = [...root.querySelectorAll("span")].find((node) => node.textContent === "Red vinyl");
      const scroller = root.querySelector(".overflow-auto");
      const header = [...root.querySelectorAll("span")].find((node) => node.textContent?.startsWith("Speed ("));
      if (!name || !scroller || !header) return null;
      const original = name.textContent;
      name.textContent = "SupercalifragilisticexpialidociousHeatTransferVinyl";
      const row = name.closest("[data-testid='cut-pass-row']");
      const field = row?.querySelector("input");
      const headerBox = header.getBoundingClientRect();
      const fieldBox = field?.getBoundingClientRect();
      const measured = {
        scrollWidth: scroller.scrollWidth,
        clientWidth: scroller.clientWidth,
        headerX: headerBox.x,
        fieldX: fieldBox?.x ?? Number.NaN,
      };
      name.textContent = original;
      return measured;
    });
    expect(nameTrack).not.toBeNull();
    expect(nameTrack!.scrollWidth).toBeLessThanOrEqual(nameTrack!.clientWidth + 1);
    expect(Math.abs(nameTrack!.headerX - nameTrack!.fieldX)).toBeLessThanOrEqual(2);
    // A 0 minimum collapsed this column once the fixed tracks filled a narrow sheet.
    // The shot stays at the desktop width either way.
    await page.setViewportSize({ width: 390, height: 800 });
    let narrow: { cell: number; headerX: number; fieldX: number } | null = null;
    try {
      narrow = await dialog.evaluate((root) => {
        const name = [...root.querySelectorAll("span")].find((node) => node.textContent === "Red vinyl");
        const header = [...root.querySelectorAll("span")].find((node) => node.textContent?.startsWith("Speed ("));
        const cell = name?.parentElement?.parentElement;
        const field = name?.closest("[data-testid='cut-pass-row']")?.querySelector("input");
        if (!name || !header || !cell || !field) return null;
        return {
          cell: cell.getBoundingClientRect().width,
          headerX: header.getBoundingClientRect().x,
          fieldX: field.getBoundingClientRect().x,
        };
      });
    } finally {
      await page.setViewportSize({ width: 1440, height: 1000 });
    }
    expect(narrow).not.toBeNull();
    expect(narrow!.cell).toBeGreaterThan(100);
    expect(Math.abs(narrow!.headerX - narrow!.fieldX)).toBeLessThanOrEqual(2);
    // A shape that starts on the mat's edge used to be the edge of this picture.
    const matBox = (await dialog.getByRole("img", { name: "Shapes in this pass" }).getAttribute("viewBox"))?.split(/[ ,]/).map(Number);
    expect(matBox?.[0]).toBeLessThan(0);
    expect(matBox?.[1]).toBeLessThan(0);
    await expect(dialog.getByRole("button", { name: "Weeding", exact: true })).toHaveAttribute("aria-expanded", "false");
    await expect(dialog.getByRole("button", { name: "Next", exact: true })).toBeDisabled();
    await expect(dialog.getByRole("button", { name: "Start Cut" })).toHaveCount(0);
    await shot(`send-step2-${theme}-invalid`);

    await fillSendLevels(page);
    await goToSendStep(page, 3);
    await expect(dialog.getByRole("img", { name: /Cut preview/ })).toBeVisible();
    await expect(dialog.getByText("Silhouette Cameo 5 Alpha").first()).toBeVisible();
    const redName = dialog.getByText("Red vinyl", { exact: true });
    const greenName = dialog.getByText("#00ff00", { exact: true });
    await expect(redName).toBeVisible();
    await expect(greenName).toBeVisible();
    // The settings used to share the name's line, and the name ellipsized to "Red …".
    // These two fit the column, so width alone would stay green with truncate put back.
    for (const name of [redName, greenName]) {
      expect(await name.evaluate((node) => node.scrollWidth > node.clientWidth + 1)).toBe(false);
      expect(await name.evaluate((node) => {
        const style = getComputedStyle(node);
        return style.textOverflow === "ellipsis" || style.whiteSpace === "nowrap";
      })).toBe(false);
    }
    const nameBox = await redName.boundingBox();
    const speedLine = dialog.getByText(/Speed 10 · force 10 · 1×/).first();
    await expect(speedLine).toBeVisible();
    const speedBox = await speedLine.boundingBox();
    if (!nameBox || !speedBox) throw new Error("pass name or its settings have no box");
    expect(speedBox.y).toBeGreaterThan(nameBox.y);
    await expect(dialog.getByRole("button", { name: "Start Cut" })).toBeEnabled();
    await shot(`send-step3-${theme}`);

    await move(2, dx);
    await dialog.getByRole("button", { name: "Start Cut" }).click();
    await expect(dialog.getByText("Your design changed.")).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Replan" })).toBeEnabled();
    // The banner is the refusal. A toast of the same one used to cover the summary card.
    await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);
    await shot(`send-stale-replan-${theme}`);
    await dialog.getByRole("button", { name: "Close" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  };

  await walk("light", 5);
  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Dark" }).click();
  await walk("dark", 12);
});

test("trace dialog: preview appears and insert adds paths", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Trace", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Trace image" })).toBeVisible();
  await expect(page.getByAltText("Traced preview")).toBeVisible();
  await expect(page.getByText("1 path")).toBeVisible();
  await page.getByRole("button", { name: "Insert" }).click();
  await expect(page.getByRole("dialog", { name: "Trace image" })).not.toBeVisible();
  // import_svg mock was invoked — it adds a node to doc, so the layer list reflects the
  // insert, same observable-effect assertion the "new doc → add rect" test above uses.
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
});

// `controlsFromSpecs` throws when the table omits a control, so that a dialog and a tracer
// disagreeing about what a trace takes is visible rather than papered over with an invented
// default. That only holds if the throw reaches the error state: it happens inside a `then`
// fulfillment handler, and a rejection handler passed to the *same* `then` does not catch it —
// the dialog would sit idle with no sliders, no error, and an unhandled rejection in the console.
test("trace dialog: a control table missing a control reports it instead of hanging", async ({ page }) => {
  await page.addInitScript(installMockTauri, { dropTraceControl: "detail" });
  await page.goto("/");
  await page.getByRole("button", { name: "Trace", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Trace image" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/detail/)).toBeVisible();
  // Nothing traceable was ever configured, so Insert must not offer geometry.
  await expect(page.getByRole("button", { name: "Insert" })).toBeDisabled();
});

test("text dialog: picking a family and Insert adds a shape", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Text", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Add text" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Font family").selectOption("Times New Roman");
  await page.getByRole("button", { name: "Insert" }).click();
  await expect(dialog).not.toBeVisible();
  // add_text mock was invoked — it adds a node to doc, same observable-effect assertion
  // the trace-insert test above uses.
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
});

test("text dialog: an empty font list says so and disables Insert", async ({ page }) => {
  await page.addInitScript(installMockTauri, { noFonts: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Text", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Add text" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("No fonts were found on this system")).toBeVisible();
  await expect(page.getByRole("button", { name: "Insert" })).toBeDisabled();
});

// The traced pane covers the common case where a file fails to decode, because both commands
// fail together. It does not cover a thumbnail that fails on its own — re-encoding the preview
// can fail while the trace succeeds — and the design spec promises every error path surfaces
// rather than turning into an empty pane.
test("trace dialog: a failed source thumbnail surfaces instead of blanking", async ({ page }) => {
  await page.addInitScript(installMockTauri, { failImagePreview: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Trace", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Trace image" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText(/broken thumbnail/)).toBeVisible();
  // The trace itself still succeeded, so the dialog stays usable.
  await expect(page.getByText("1 path")).toBeVisible();
  await expect(page.getByRole("button", { name: "Insert" })).toBeEnabled();
});

// The canvas's pixels are unreadable from a test, so these turn document mm into page px with the
// view the canvas publishes as `data-view="scale tx ty"`.
type CanvasView = { scale: number; tx: number; ty: number };

async function readView(page: Page): Promise<CanvasView> {
  const attr = (await page.getByTestId("design-canvas").getAttribute("data-view")) ?? "";
  const [scale, tx, ty] = attr.split(" ").map(Number);
  return { scale, tx, ty };
}

async function fittedView(page: Page): Promise<CanvasView> {
  await expect(page.getByTestId("design-canvas")).not.toHaveAttribute("data-view", "1 0 0");
  return readView(page);
}

async function toPage(page: Page, v: CanvasView, mm: { x: number; y: number }) {
  const box = await page.getByTestId("design-canvas").boundingBox();
  if (!box) throw new Error("design canvas has no layout box");
  return { x: box.x + mm.x * v.scale + v.tx, y: box.y + mm.y * v.scale + v.ty };
}

/** The Cameo bed these wheel deltas were written against. Fit used to show that bed (~0.2 px/mm)
 *  with a 24 px margin and no rulers; it now shows the mat, and the same notch count walks the
 *  shapes off the canvas. 24 is that old margin, not the mat fit's padding. */
const CAMEO_BED_MM = { w: 330, h: 3000 };
const BED_FIT_MARGIN_PX = 24;

/** Ctrl-wheel at (the whole pixel nearest) a document point; returns the view once the zoom has
 *  landed. Whole pixels because Chromium reports a wheel's position as integers: a fractional
 *  target puts the pinned point up to a pixel away, and a 20× zoom turns that into twenty.
 *  The delta is applied to the old bed-fit scale, not the current mat fit, so a drag of
 *  `n * v.scale` still means n millimetres on the canvas. */
async function zoomInAt(page: Page, mm: { x: number; y: number }, deltaY: number): Promise<CanvasView> {
  const before = await fittedView(page);
  const box = await page.getByTestId("design-canvas").boundingBox();
  if (!box) throw new Error("design canvas has no layout box");
  const availW = Math.max(box.width - 2 * BED_FIT_MARGIN_PX, 1);
  const availH = Math.max(box.height - 2 * BED_FIT_MARGIN_PX, 1);
  const bedFit = Math.min(availW / CAMEO_BED_MM.w, availH / CAMEO_BED_MM.h);
  const target = bedFit * Math.exp(-deltaY * 0.01);
  // Already at least as close as this delta used to get (the mat fit is past a shallow zoom).
  if (target <= before.scale * 1.01) return before;
  const at = await toPage(page, before, mm);
  await page.mouse.move(Math.round(at.x), Math.round(at.y));
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, Math.round(-Math.log(target / before.scale) / 0.01));
  await page.keyboard.up("Control");
  await expect.poll(async () => (await readView(page)).scale).toBeGreaterThan(before.scale);
  return readView(page);
}

test("Ctrl-wheel zooms about the cursor and the status bar reports it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v0 = await fittedView(page);
  const zoom = page.getByTestId("status-zoom");
  const before = parseInt((await zoom.textContent()) ?? "", 10);
  const target = await toPage(page, v0, { x: 5, y: 5 });
  const cursor = { x: Math.round(target.x), y: Math.round(target.y) }; // where zoomInAt puts it
  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  const under = { x: (cursor.x - box.x - v0.tx) / v0.scale, y: (cursor.y - box.y - v0.ty) / v0.scale };

  const v1 = await zoomInAt(page, { x: 5, y: 5 }, -300);

  await expect.poll(async () => parseInt((await zoom.textContent()) ?? "", 10)).toBeGreaterThan(before);
  // About the cursor: the document point that was under it is still under it.
  const after = await toPage(page, v1, under);
  expect(after.x).toBeCloseTo(cursor.x, 0);
  expect(after.y).toBeCloseTo(cursor.y, 0);
});

test("dragging a corner handle commits one scale about the opposite corner", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // the red 10 × 10 mm rect at the origin
  // Far enough in that the handles are tens of px apart.
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  const se = await toPage(page, v, { x: 10, y: 10 });
  await page.mouse.move(se.x, se.y);
  await page.mouse.down();
  await page.mouse.move(se.x + 10 * v.scale, se.y + 5 * v.scale, { steps: 4 });
  await page.mouse.up();

  const commits = await page.evaluate(
    () => (window as unknown as { __commitTransforms?: { ids: number[]; m: number[] }[] }).__commitTransforms ?? [],
  );
  expect(commits).toHaveLength(1); // one gesture, one commit, one undo entry
  expect(commits[0].ids).toEqual([2]);
  // 10 → 20 mm wide, 10 → 15 mm tall, with the nw corner at the origin held still.
  const [a, b, c, d, e, f] = commits[0].m;
  expect(a).toBeCloseTo(2, 1);
  expect(d).toBeCloseTo(1.5, 1);
  for (const zero of [b, c, e, f]) expect(zero).toBeCloseTo(0, 1);
});

test("a marquee over both shapes selects both", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  // Zoomed in so the band's start point is clear of the shapes' hit tolerance. The band runs from
  // empty space below-right up to a point inside both shapes: the shapes sit at the artboard's top
  // edge, so after zooming there is almost no canvas above them, and a band only has to touch.
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  const from = await toPage(page, v, { x: 20, y: 20 });
  const to = await toPage(page, v, { x: 4, y: 4 });
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 5 });
  await page.mouse.up();

  await expect(page.locator('[data-testid="layer-row"][data-selected="true"]')).toHaveCount(2);
});

test("the cursor readout follows a wheel pan under a still pointer", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v0 = await fittedView(page);
  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  const pointer = { x: Math.round(box.x + box.width / 2), y: Math.round(box.y + box.height / 2) };
  await page.mouse.move(pointer.x, pointer.y);
  const readY = async () => Number(/y (-?[\d.]+) mm/.exec((await page.getByTestId("status-cursor").textContent()) ?? "")?.[1]);
  const before = await readY();

  await page.mouse.wheel(0, 100); // scroll down: the view pans up, the pointer is over a lower point
  await expect.poll(async () => (await readView(page)).ty).toBeLessThan(v0.ty);

  // A readout stored at the last pointer move would still say `before`; the world moved under it.
  await expect.poll(readY).toBeCloseTo(before + 100 / v0.scale, 0);
});

// One transform on the wire at a time. A second gesture pressed while the first commit is
// unanswered would compute its matrix from the first one's preview; if the first were then
// refused, the second would land about the wrong anchor (CodeRabbit on #298).
test("a drag pressed while the previous commit is in flight does not commit", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const commits = () =>
    page.evaluate(() => ((window as unknown as { __commitTransforms?: unknown[] }).__commitTransforms ?? []).length);
  const drag = async (from: { x: number; y: number }, dx: number) => {
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(from.x + dx, from.y, { steps: 3 });
    await page.mouse.up();
  };

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  const centre = await toPage(page, v, { x: 5, y: 5 });
  await drag(centre, 5 * v.scale); // move right 5 mm; its commit is now parked
  await expect.poll(commits).toBe(1);

  await drag({ x: centre.x + 5 * v.scale, y: centre.y }, 5 * v.scale); // press on the preview
  expect(await commits()).toBe(1);

  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
  await drag({ x: centre.x + 5 * v.scale, y: centre.y }, 5 * v.scale); // settled: drags commit again
  await expect.poll(commits).toBe(2);
});

type CommitRecord = { ids: number[]; m: number[]; batch?: number };

async function commitLog(page: Page): Promise<CommitRecord[]> {
  return page.evaluate(() => (window as unknown as { __commitTransforms?: CommitRecord[] }).__commitTransforms ?? []);
}

async function dragBy(page: Page, from: { x: number; y: number }, dx: number, dy: number) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + dx, from.y + dy, { steps: 4 });
  await page.mouse.up();
}

/** The fake's own transform for a node, read through the same command the app uses. */
async function nodeTransform(page: Page, id: number): Promise<number[]> {
  const json = await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("snapshot", {}),
  );
  return (JSON.parse(json as string) as { nodes: Record<string, { transform: number[] }> }).nodes[id].transform;
}

test("a refused transform puts the shape back and the next drag starts from where it was", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const centre = await toPage(page, v, { x: 5, y: 5 });

  await page.evaluate(() => (window as unknown as { __failNextCommit: () => void }).__failNextCommit());
  await dragBy(page, centre, 5 * v.scale, 0);
  await expect(page.getByText("transform refused")).toBeVisible();

  // Pressed where the shape was: a preview left stranded would put the box 5 mm to the right.
  await dragBy(page, centre, 5 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(5, 0);
});

test("a transform that lands but cannot be re-read keeps the shape where the backend has it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const centre = await toPage(page, v, { x: 5, y: 5 });

  await page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await dragBy(page, centre, 20 * v.scale, 0); // the commit lands; its refresh fails
  await expect(page.getByText(/Edit applied, but the canvas could not be refreshed/)).toBeVisible();

  // The box is where the backend put it (20 mm right), clear of where the shape used to be: a
  // revert to the stale scene would make this press miss and start a marquee instead.
  await dragBy(page, { x: centre.x + 20 * v.scale, y: centre.y }, 5 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(25, 0);
});

test("an edit keeps the operator's zoom; Ctrl+0 and a machine switch refit", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const fitted = await fittedView(page);
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  // The snapshot after the commit rebuilds the artboard object; the view must not refit for it.
  expect(await readView(page)).toEqual(v);

  await page.getByTestId("design-canvas").hover();
  await page.keyboard.press("Control+0");
  await expect.poll(() => readView(page)).toEqual(fitted);

  await showTab(page, "Properties");
  await page.getByLabel("Design for").selectOption("puma");
  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  // Puma is roll-only. The default roll is the machine width by 36 in. Rulers take 24 px off the
  // left and top, the zoom bar takes 54 px off the bottom, and the mat fit pads 32 px, so the
  // scale is not the 600 × 5000 mm bed.
  const margin = 32;
  const ruler = 24;
  const zoomBar = 54;
  const rollW = 600;
  const rollH = 36 * 25.4;
  const availW = box.width - 2 * margin - ruler;
  const availH = box.height - 2 * margin - ruler - zoomBar;
  await expect.poll(async () => (await readView(page)).scale).toBeCloseTo(Math.min(availW / rollW, availH / rollH), 3);
});

test("Command-A selects every shape on the mat and leaves the chrome and a focused field alone", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedAlignExtras: true });
  await page.goto("/");
  await showTab(page, "Layers");
  const rows = page.getByTestId("layer-row");
  // Two loose rects, a group of two rects, and an empty group.
  await expect(rows).toHaveCount(6);
  await rows.first().click();
  const selectedRows = page.locator('[data-testid="layer-row"][data-selected="true"]');
  await expect(selectedRows).toHaveCount(1);
  await expect(page.getByText("10.0 × 10.0 mm")).toBeVisible();

  // The chord belongs to the field. Stealing it would select the other three shapes.
  await showTab(page, "Properties");
  const x = page.getByLabel("X", { exact: true });
  await x.focus();
  // The platform chord: Ctrl+A on macOS moves the caret, so the field would keep its "0".
  await page.keyboard.press("ControlOrMeta+A");
  // Replacing the field's "0" commits x = 4. The other rect stays at the origin, so the
  // union below is still 0..80 × 0..50.
  await x.press("4");
  await expect(x).toHaveValue("4");
  await showTab(page, "Layers");
  await expect(selectedRows).toHaveCount(1);
  // Shift stays free. The one selected shape must not become every shape.
  await page.keyboard.press("Shift+Meta+A");
  await expect(selectedRows).toHaveCount(1);

  // Focus stays on the toolbar button, the same way keyboard zoom is reached.
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.keyboard.press("Meta+A");
  // The two grouped rects sit at 50,20 and 70,40, so the box is the whole mat, 0..80 × 0..50.
  await expect(page.getByText("80.0 × 50.0 mm")).toBeVisible();
  await expect(selectedRows).toHaveCount(4);
  const groups = rows.filter({ hasText: "Group" });
  await expect(groups).toHaveCount(2);
  await expect(groups.nth(0)).toHaveAttribute("data-selected", "false");
  await expect(groups.nth(1)).toHaveAttribute("data-selected", "false");
  expect(await page.evaluate(() => window.getSelection()?.toString() ?? "")).toBe("");

  await rows.first().click();
  await expect(selectedRows).toHaveCount(1);
  await page.keyboard.press("Control+A");
  await expect(selectedRows).toHaveCount(4);
  expect(await page.evaluate(() => window.getSelection()?.toString() ?? "")).toBe("");
});

test("Command-A on an empty mat leaves the chrome unselected", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.keyboard.press("Meta+A");
  await page.keyboard.press("Control+A");
  await expect(page.getByText("No selection")).toBeVisible();
  await showTab(page, "Properties");
  await expect(page.getByText("Select an object to edit its size and position.")).toBeVisible();
  expect(await page.evaluate(() => window.getSelection()?.toString() ?? "")).toBe("");
});

test("keyboard zoom works after a toolbar click and leaves a focused field alone", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const fitted = await fittedView(page);
  await page.getByRole("button", { name: "Select" }).click(); // focus stays on the button

  await page.keyboard.press("Control+=");
  await expect.poll(async () => (await readView(page)).scale).toBeCloseTo(fitted.scale * 1.25, 6);
  await page.keyboard.press("Control+1");
  await expect(page.getByTestId("status-zoom")).toHaveText("100%");
  await page.keyboard.press("Control+0");
  await expect.poll(() => readView(page)).toEqual(fitted);

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).focus();
  await page.keyboard.press("Control+=");
  expect(await readView(page)).toEqual(fitted);
});

test("Shift-click inside the selection toggles a shape out without dragging the rest", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  await dragBy(page, await toPage(page, v, { x: 20, y: 20 }), -16 * v.scale, -16 * v.scale); // marquee both
  const selectedRows = page.locator('[data-testid="layer-row"][data-selected="true"]');
  await expect(selectedRows).toHaveCount(2);

  // Both seeded rects share the origin, so this lands on the topmost of the two.
  await page.keyboard.down("Shift");
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0);
  await page.keyboard.up("Shift");

  await expect(selectedRows).toHaveCount(1);
  expect(await commitLog(page)).toEqual([]);
});

test("Space-drag pans after a toolbar click without pressing the button, and so does a middle drag", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const beforeAdd = await fittedView(page);
  await page.getByRole("button", { name: "Rectangle", exact: true }).click(); // adds one; focus stays on it
  await showTab(page, "Layers");
  const rows = page.getByTestId("layer-row");
  await expect(rows).toHaveCount(3);
  // Adding a shape fits the artwork, so the pan is measured from that view, not the mat fit.
  await expect.poll(async () => Math.abs((await readView(page)).scale - beforeAdd.scale)).toBeGreaterThan(0.05);
  const v0 = await readView(page);
  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  const start = { x: Math.round(box.x + box.width / 2), y: Math.round(box.y + box.height / 2) };

  await page.mouse.move(start.x, start.y);
  await page.keyboard.down("Space");
  await dragBy(page, start, 50, 30);
  await page.keyboard.up("Space");
  await expect.poll(async () => (await readView(page)).tx).toBeCloseTo(v0.tx + 50, 0);
  expect((await readView(page)).ty).toBeCloseTo(v0.ty + 30, 0);
  await expect(rows).toHaveCount(3); // Space never reached the Rectangle button
  expect(await commitLog(page)).toEqual([]);

  await page.mouse.move(start.x, start.y);
  await page.mouse.down({ button: "middle" });
  await page.mouse.move(start.x - 20, start.y, { steps: 3 });
  await page.mouse.up({ button: "middle" });
  await expect.poll(async () => (await readView(page)).tx).toBeCloseTo(v0.tx + 30, 0);
});

test("vinyl colours stay distinct on the mat in both themes, including a gold outline inside red", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(installMockTauri, { seedVinylBoard: true });
  await page.goto("/");
  await fittedView(page);
  await expect(page.getByRole("button", { name: "Fit mat" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Fit content" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Zoom in" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Zoom out" })).toBeVisible();

  const readBoard = () =>
    page.evaluate(() => {
      const canvas = document.querySelector("[data-testid='design-canvas']") as HTMLCanvasElement | null;
      if (!canvas) return null;
      const [scale, tx, ty] = (canvas.getAttribute("data-view") ?? "").split(" ").map(Number);
      const dpr = canvas.width / canvas.clientWidth;
      const ctx = canvas.getContext("2d");
      if (!ctx || !(scale > 0)) return null;
      const at = (x: number, y: number) => {
        const sx = Math.round((x * scale + tx) * dpr);
        const sy = Math.round((y * scale + ty) * dpr);
        const px = ctx.getImageData(sx, sy, 1, 1).data;
        return [px[0], px[1], px[2]];
      };
      const along: { x: number; rgb: number[] }[] = [];
      for (let x = 50.8; x <= 100; x += 0.2) along.push({ x, rgb: at(x, 50.8) });
      return {
        along,
        mat: at(250, 250),
        fills: {
          black: at(130.24, 25.24),
          white: at(175.24, 25.24),
          navy: at(220.24, 25.24),
          purple: at(130.24, 70.24),
          pale: at(175.24, 70.24),
        },
      };
    });

  const farFrom = (a: number[], b: number[]) => Math.max(Math.abs(a[0] - b[0]), Math.abs(a[1] - b[1]), Math.abs(a[2] - b[2]));
  for (const theme of ["Light", "Dark"] as const) {
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: theme }).click();
    await expect.poll(async () => documentDark(page)).toBe(theme === "Dark");
    // The mat pixel is the redraw, not just the class: the canvas keeps the previous frame until draw().
    await expect.poll(async () => (await readBoard())?.mat[0] ?? -1).toBeLessThan(theme === "Dark" ? 120 : 255);
    await expect.poll(async () => (await readBoard())?.mat[0] ?? -1).toBeGreaterThan(theme === "Dark" ? 20 : 160);

    const board = await readBoard();
    expect(board, theme).not.toBeNull();
    if (!board) continue;
    const gold = board.along.find((s) => s.x > 70 && s.x < 78 && isGoldPixel(s.rgb));
    const red = board.along.find((s) => s.x > 82 && isRedPixel(s.rgb));
    expect(gold, `${theme} gold outline ${JSON.stringify(board.along.filter((s) => s.x > 68 && s.x < 80))}`).toBeTruthy();
    expect(red, `${theme} red fill`).toBeTruthy();
    for (const [name, rgb] of Object.entries(board.fills)) {
      expect(farFrom(rgb, board.mat), `${theme} ${name} ${rgb.join(",")}`).toBeGreaterThan(15);
    }
    const seen = Object.values(board.fills);
    for (let i = 0; i < seen.length; i++) {
      for (let j = i + 1; j < seen.length; j++) {
        expect(farFrom(seen[i], seen[j]), theme).toBeGreaterThan(12);
      }
    }
    await page.screenshot({ path: `/opt/cursor/artifacts/editor-${theme.toLowerCase()}.png` }).catch(() => undefined);
    if (theme === "Dark") {
      await page.getByRole("tablist").screenshot({ path: "/opt/cursor/artifacts/tabs-dark.png" }).catch(() => undefined);
    }
  }

  const box = (await page.getByTestId("design-canvas").boundingBox())!;
  const v = await readView(page);
  const mat = 12 * 25.4;
  const matBottom = box.y + v.ty + mat * v.scale;
  const matLeft = box.x + v.tx;
  const matRight = matLeft + mat * v.scale;
  const bar = (await page.getByTestId("zoom-controls").boundingBox())!;
  expect(matLeft - box.x).toBeGreaterThanOrEqual(24 + 32 - 1);
  expect(box.y + v.ty - box.y).toBeGreaterThanOrEqual(24 + 32 - 1);
  expect(matBottom).toBeLessThanOrEqual(bar.y + 1);
  expect(matRight).toBeLessThanOrEqual(box.x + box.width - 32 + 1);
  const rulers = page.getByRole("button", { name: "Rulers" });
  await expect(rulers).toHaveAttribute("aria-pressed", "true");
});

async function documentDark(page: Page): Promise<boolean> {
  return page.evaluate(() => document.documentElement.classList.contains("dark"));
}

function isGoldPixel(rgb: number[]): boolean {
  const [r, g, b] = rgb;
  return r > 80 && g > 40 && b < 90 && r > b + 40 && g > b + 25;
}

function isRedPixel(rgb: number[]): boolean {
  const [r, g, b] = rgb;
  return r > 150 && r > g + 40 && r > b + 40 && g < 160;
}

test("dragging just outside a corner rotates about the centre; Alt scales about it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const c = await toPage(page, v, { x: 5, y: 5 });
  const se = await toPage(page, v, { x: 10, y: 10 });

  // Inside the rotate zone (18 px) and outside the handle (6 px), a quarter turn clockwise.
  const from = { x: se.x + 8, y: se.y + 8 };
  const r = Math.hypot(from.x - c.x, from.y - c.y);
  const to = { x: c.x + r * Math.cos((3 * Math.PI) / 4), y: c.y + r * Math.sin((3 * Math.PI) / 4) };
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(to.x, to.y, { steps: 8 });
  await page.mouse.up();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [a, b, cc, d, e, f] = (await commitLog(page))[0].m;
  expect([a, b, cc, d]).toEqual([expect.closeTo(0, 1), expect.closeTo(1, 1), expect.closeTo(-1, 1), expect.closeTo(0, 1)]);
  // About the centre (5, 5): it maps to itself.
  expect(a * 5 + cc * 5 + e).toBeCloseTo(5, 0);
  expect(b * 5 + d * 5 + f).toBeCloseTo(5, 0);
});

test("Alt-dragging a corner scales about the centre", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const se = await toPage(page, v, { x: 10, y: 10 });

  await page.keyboard.down("Alt");
  await dragBy(page, se, 10 * v.scale, 5 * v.scale);
  await page.keyboard.up("Alt");

  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  // Half-widths 5 → 15 and 5 → 10 about (5, 5): ×3 and ×2 with the centre fixed.
  const [a, b, c, d, e, f] = (await commitLog(page))[0].m;
  expect([a, b, c, d, e, f]).toEqual([3, 0, 0, 2, -10, -5].map((x) => expect.closeTo(x, 1)));
});

// The gesture follows the document point under the pointer, not the pointer's last world position:
// a pan under a held, still pointer must move the shape with it (CodeRabbit on #298).
test("a wheel pan during a drag carries the shape with the pointer and commits that", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  const centre = await toPage(page, v, { x: 5, y: 5 });

  await page.mouse.move(centre.x, centre.y);
  await page.mouse.down();
  await page.mouse.move(centre.x + 5 * v.scale, centre.y, { steps: 3 });
  await page.mouse.wheel(0, 100); // the view pans up 100 px; the pointer now sits 100 px lower in mm
  await expect.poll(async () => (await readView(page)).ty).toBeLessThan(v.ty);
  await page.mouse.up();

  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [, , , , e, f] = (await commitLog(page))[0].m;
  expect(e).toBeCloseTo(5, 0);
  expect(f).toBeCloseTo(100 / v.scale, 0);
});

test("a Group selected in the layers panel gets a box around its shapes and commits as itself", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedGroup: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").filter({ hasText: "Group" }).click();
  const v = await zoomInAt(page, { x: 35, y: 5 }, -350);

  // The box spans the Group's rect, 30..40 mm; drag its se corner out by 10 × 5 mm.
  await dragBy(page, await toPage(page, v, { x: 40, y: 10 }), 10 * v.scale, 5 * v.scale);

  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([1 + 1]); // the Group (id 2, after the root), not its rect
  // Width 10 → 20 and height 10 → 15 about the nw corner (30, 0).
  const [a, b, c, d, e, f] = m;
  expect([a, b, c, d, e, f]).toEqual([2, 0, 0, 1.5, -30, 0].map((x) => expect.closeTo(x, 1)));
});

// The X/Y/W/H fields are a transform producer too: they must compute from the geometry the canvas
// shows (an unread commit's preview included) and wait their turn behind a commit on the wire, or
// they send a matrix built on the position the shape has already left (Copilot on #298).
test("a property edit after an applied-but-unread commit starts from where the shape now is", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 20 * v.scale, 0); // lands at x = 20; refresh fails
  await expect(page.getByText(/Edit applied, but the canvas could not be refreshed/)).toBeVisible();

  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("5");
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1].m[4]).toBeCloseTo(-15, 1);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(5, 1);
});

test("a property edit made while a commit is in flight waits for it, then lands where it says", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 20 * v.scale, 0); // parked on the wire
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("5");
  expect((await commitLog(page)).length).toBe(1); // queued, not sent alongside

  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(5, 1);
});

test("edits to two different fields made during an in-flight commit both land", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 20 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  // Different fields do not supersede each other: a single queue slot dropped X when Y arrived
  // (CodeRabbit and Copilot on #298).
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("5");
  await showTab(page, "Properties");
  await page.getByLabel("Y", { exact: true }).fill("7");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  await expect.poll(async () => (await nodeTransform(page, 2)).slice(4)).toEqual([expect.closeTo(5, 1), expect.closeTo(7, 1)]);
});

// The fake has to mirror transform_nodes for nested nodes (Copilot on #298): a scale committed on
// a Group's child lands in the parent's space, not as if the child sat at the root.
test("scaling a shape inside a moved Group keeps it where the real backend would", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedGroup: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").filter({ hasText: "Rectangle" }).click(); // the Group's rect
  const v = await zoomInAt(page, { x: 35, y: 5 }, -350);

  await dragBy(page, await toPage(page, v, { x: 40, y: 10 }), 10 * v.scale, 5 * v.scale);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  // World: ×2, ×1.5 about (30, 0). In the Group's space, which is translated by 30, that is a pure
  // scale about its origin.
  await expect.poll(async () => nodeTransform(page, 3)).toEqual([2, 0, 0, 1.5, 0, 0].map((x) => expect.closeTo(x, 1)));
});

// Queued edits drain one per settled commit, each built from the geometry the one before it left:
// W scales about where X put the shape, not where the drag did (Copilot on #298). They drain after
// the snapshot before them has rendered, so this no longer shows a snapshot rendering while a commit
// is on the wire; the test after it does.
test("queued X then W behind a move scale about where X put the shape", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 20 * v.scale, 0); // to x = 20, parked
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("5");
  await showTab(page, "Properties");
  await page.getByLabel("W", { exact: true }).fill("20");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The 10 mm rect ends at x = 5, 20 mm wide: scale ×2 about x = 5, so e = 5 and a = 2.
  await expect.poll(async () => (await commitLog(page)).length).toBe(3);
  await expect.poll(async () => {
    const [a, , , , e] = await nodeTransform(page, 2);
    return [a, e];
  }).toEqual([expect.closeTo(2, 1), expect.closeTo(5, 1)]);
});

// A snapshot can render while a commit is on the wire, one the backend answered before the commit
// arrived: a run() command's refresh. It must not retire that commit's preview, or the canvas and
// fields jump back and the next edit is built from geometry the backend has left (Copilot on #298).
// The fake runs a held commit on release, so the machine switch's snapshot here is that older one.
test("a snapshot rendered while a commit is on the wire keeps that commit's preview", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red, at 0
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("20"); // on the wire
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  const before = (await readView(page)).scale;
  await showTab(page, "Properties");
  await page.getByLabel("Design for").selectOption("puma"); // its refresh renders a new document now
  await expect.poll(async () => (await readView(page)).scale).not.toBeCloseTo(before, 6);
  await showTab(page, "Properties");
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("20");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(20, 6);
});

// Snapping (spec 2026-10-07). With both seeds: the Group's rect at 30..40 mm (id 3) and the red
// rect (id 4) and green rect at 0..10 mm. Layer rows run Group, its rect, red, green.
async function selectRedBesideGroup(page: Page): Promise<CanvasView> {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(2).click();
  return zoomInAt(page, { x: 20, y: 5 }, -350);
}

test("a move that stops just short of another shape snaps flush against it", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  // Right edge would stop at 29.6 mm, 0.4 mm short of the Group's rect: within reach, so it butts.
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 19.6 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([4]);
  expect(m[4]).toBeCloseTo(20, 2);
});

test("holding Ctrl drags without snapping", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.keyboard.down("Control");
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 19.6 * v.scale, 0);
  await page.keyboard.up("Control");
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0].m[4]).toBeCloseTo(19.6, 1);
});

test("an edge handle that stops just short of another shape snaps its edge to it", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await dragBy(page, await toPage(page, v, { x: 10, y: 5 }), 19.6 * v.scale, 0); // the e handle
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [a, , , , e] = (await commitLog(page))[0].m;
  // The right edge goes 10 → 30 mm (snapped from 29.6), so the 10 mm rect becomes 30 wide: ×3, not
  // the 2.96 an unsnapped drag would give, with its left edge fixed at 0.
  expect(a).toBeCloseTo(3, 2);
  expect(e).toBeCloseTo(0, 2);
});

test("a selected Group does not snap to its own shape", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click(); // the Group; its rect is excluded from targets
  const v = await zoomInAt(page, { x: 35, y: 5 }, -350);
  // 0.3 mm is well within reach of where the Group's rect started; a self-snap would pull it back.
  await dragBy(page, await toPage(page, v, { x: 35, y: 5 }), 0.3 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0].m[4]).toBeCloseTo(0.3, 1);
});

test("pressing on an unselected shape and dragging snaps too", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  const v = await zoomInAt(page, { x: 20, y: 5 }, -350);
  // Nothing selected: the press picks the topmost rect at the origin (green, id 5) and drags it.
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 19.6 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([5]);
  expect(m[4]).toBeCloseTo(20, 2);
});

test("a move can snap to the artboard's centre line", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click(); // the Group, centred at x = 35
  const v = await zoomInAt(page, { x: 35, y: 5 }, -200); // ~1.5 px/mm, so the reach is ~4 mm
  // Its centre would stop at 164.6 mm; the Cameo's 330 mm bed is centred at 165, a line only the
  // artboard has.
  const from = await toPage(page, v, { x: 35, y: 5 });
  await dragBy(page, from, 129.6 * v.scale, 0);
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0].m[4]).toBeCloseTo(130, 1);
});

test("an Alt edge scale snaps the dragged edge and mirrors the other", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.keyboard.down("Alt");
  await dragBy(page, await toPage(page, v, { x: 10, y: 5 }), 19.6 * v.scale, 0); // e handle
  await page.keyboard.up("Alt");
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  // About the centre (5): the right edge snaps 29.6 → 30, so the half-width 5 becomes 25 (×5), and
  // the left edge mirrors to −20.
  const [a, , , , e] = (await commitLog(page))[0].m;
  expect(a).toBeCloseTo(5, 2);
  expect(e).toBeCloseTo(-20, 1);
});

// Align and distribute (spec 2026-10-07). With both seeds: the Group (id 2) holds a rect at 30..40
// mm, and the red (id 4) and green (id 5) rects sit at 0..10 mm. Layer rows run Group, its rect,
// red, green.
async function selectRows(page: Page, rows: number[]) {
  const [first, ...rest] = rows;
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(first).click();
  await showTab(page, "Layers");
  for (const r of rest) await page.getByTestId("layer-row").nth(r).click({ modifiers: ["Shift"] });
}

test("align left on two rects and a Group commits one batch that moves only the Group", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [2, 3, 0]);
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click();

  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m, batch }] = await commitLog(page);
  expect(ids).toEqual([2]);
  expect(m).toEqual([1, 0, 0, 1, -30, 0]);
  expect(batch).toBe(1);
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(0, 6);
});

test("align on a single rect centres it on the artboard", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [2]);
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align horizontal centres" }).click();

  // The 330 mm bed is centred at 165; the rect's centre is at 5.
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([4]);
  expect(m[4]).toBeCloseTo(160, 6);
  expect(m[5]).toBe(0);
});

test("distribute equalises the gaps and breaks a tie by document order, not click order", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [0, 3, 2]); // Group, green, red: the reverse of document order
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Distribute horizontal spacing" }).click();

  // Span 0..40 holding 30 mm of shapes leaves two 5 mm gaps. Red and green both start at 0, so
  // document order makes red first and green the one that moves, to 15.
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  const [{ ids, m }] = await commitLog(page);
  expect(ids).toEqual([5]);
  expect(m[4]).toBeCloseTo(15, 6);
});

test("align and distribute are disabled when they cannot act", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await showTab(page, "Properties");
  // Nothing selected is the document, not a disabled align row. The buttons come back with a selection.
  await expect(page.getByRole("heading", { name: "Document", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Align left edges" })).toHaveCount(0);

  // The Group, its own rect and red are two units, since the rect moves with the Group: still too
  // few to distribute.
  await selectRows(page, [0, 1, 2]);
  const alignLeft = page.getByRole("button", { name: "Align left edges" });
  const distribute = page.getByRole("button", { name: "Distribute horizontal spacing" });
  // Selecting rows shows Layers, which unmounts these buttons. They follow the selection once
  // Properties is showing again.
  await showTab(page, "Properties");
  await expect(alignLeft).toBeEnabled();
  await expect(distribute).toBeDisabled();

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(3).click({ modifiers: ["Shift"] });
  await showTab(page, "Properties");
  await expect(distribute).toBeEnabled();
});

test("an align clicked while a drag's commit is on the wire lines up from where the drag left", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] });
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click();
  expect((await commitLog(page)).length).toBe(1); // queued behind the drag, not sent
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // From the stale scene the Group would go to 0 (−30); from the drag's result it goes to 5.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  const { ids, m } = (await commitLog(page))[1];
  expect(ids).toEqual([2]);
  expect(m[4]).toBeCloseTo(-25, 6);
});

// With seedAlignExtras beside the rects: red (id 2) and green (id 3) at 0..10, the two-rect Group
// (id 4) spanning 50..80 × 20..50, and the empty Group (id 7). Rows run red, green, Group, its two
// rects, the empty Group.
async function seedAlignExtras(page: Page) {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedAlignExtras: true });
  await page.goto("/");
}

test("a Group aligns by the bounds of all its shapes, on both axes", async ({ page }) => {
  await seedAlignExtras(page);
  await selectRows(page, [0, 2]);

  // The Group's right edge is its second rect's, at 80; its first rect alone would say 60.
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align right edges" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0]).toMatchObject({ ids: [2], m: [1, 0, 0, 1, 70, 0] });
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(70, 6);

  // Its bottom is 50, again from the second rect.
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align bottom edges" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1]).toMatchObject({ ids: [2], m: [1, 0, 0, 1, 0, 40] });
});

test("distribute vertical spacing moves the middle unit on y only", async ({ page }) => {
  await seedAlignExtras(page);
  await selectRows(page, [0, 1, 2]);
  // Red and green at 0..10, the Group at 20..50: span 0..50 holds 50 mm, so the gaps are 0 and
  // green, second in document order, goes to 10.
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Distribute vertical spacing" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0]).toMatchObject({ ids: [3], m: [1, 0, 0, 1, 0, 10] });
});

test("an empty Group is not a unit, so it does not enable distribute", async ({ page }) => {
  await seedAlignExtras(page);
  await selectRows(page, [0, 1, 5]);
  await showTab(page, "Properties");
  await expect(page.getByRole("button", { name: "Distribute horizontal spacing" })).toBeDisabled();
  await showTab(page, "Properties");
  await expect(page.getByRole("button", { name: "Distribute vertical spacing" })).toBeDisabled();
  await showTab(page, "Properties");
  await expect(page.getByRole("button", { name: "Align left edges" })).toBeEnabled();
});

test("a refused align puts every unit back and the next click starts from there", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [2, 0]); // red and the Group: centres 5 and 35, so both move to 20
  await page.evaluate(() => (window as unknown as { __failNextCommit: () => void }).__failNextCommit());
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align horizontal centres" }).click();
  await expect(page.getByText("transform refused")).toBeVisible();

  // A preview left in place would have both centred already, and this click would send nothing.
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align horizontal centres" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  const log = await commitLog(page);
  expect(log.map((e) => [e.ids[0], e.m[4], e.batch])).toEqual([[2, -15, 1], [4, 15, 1]]); // document order
});

test("the fake refuses a whole batch that names a missing node", async ({ page }) => {
  // The fake is what the frontend is tested against, so it must not half-apply what the backend
  // refuses outright.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  const refused = await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("commit_transforms", { moves: [
        { ids: [4], m: [1, 0, 0, 1, 5, 0] },
        { ids: [999], m: [1, 0, 0, 1, 1, 0] },
      ] }).then(() => false, () => true),
  );
  expect(refused).toBe(true);
  expect(await commitLog(page)).toEqual([]);
  expect(await nodeTransform(page, 4)).toEqual([1, 0, 0, 1, 0, 0]);
});

test("queued aligns replace each other per axis, not across axes", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 5 * v.scale); // red to 5..15 × 5..15
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] });
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click();
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align top edges" }).click();
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align right edges" }).click(); // replaces left, not top
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // Top takes red up to the Group's 0; right takes red to the Group's 40. Left (the Group to 5)
  // never runs.
  await expect.poll(async () => (await commitLog(page)).length).toBe(3);
  const [, top, right] = await commitLog(page);
  expect(top).toMatchObject({ ids: [4], m: [1, 0, 0, 1, 0, expect.closeTo(-5, 6)] });
  expect(right).toMatchObject({ ids: [4], m: [1, 0, 0, 1, expect.closeTo(25, 6), 0] });
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 30, 0]);
});

test("an align that would move nothing sends nothing and does not hold the next one", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [2, 3]); // red and green, both already at 0
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click();
  // Never invoked, not just never logged: the fake refuses an empty batch before it logs anything,
  // so an empty send would pass the log check below and put a refusal on screen.
  expect(await page.evaluate(() => (window as unknown as { __maxInFlightCommits?: number }).__maxInFlightCommits)).toBeUndefined();
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] });
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0]).toMatchObject({ ids: [2], batch: 1 });
});

test("a queued distribute does not replace a queued align on the same axis", async ({ page }) => {
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(3).click({ modifiers: ["Shift"] }); // green
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] }); // the Group
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Distribute horizontal spacing" }).click();
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // Distribute first: green 0..10 and the Group 30..40 stay, red goes to 15 (+10). Then align left
  // from there: red −15, the Group −30. Align alone would have sent red −5.
  await expect.poll(async () => (await commitLog(page)).length).toBe(4);
  const log = await commitLog(page);
  expect(log.slice(1).map((e) => [e.ids[0], Math.round(e.m[4] * 1e6) / 1e6, e.batch])).toEqual([[4, 10, 2], [2, -30, 3], [4, -15, 3]]);
});

test("an align's preview moves every unit while its commit is on the wire", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [0, 2, 3]); // the Group, red, green: the centre of 0..40 is 20
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align horizontal centres" }).click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(3);

  // The fields read the effective scene, so they show the preview: red and green both at 15. A
  // preview that applied only the first move (the Group's) would leave them at 0.
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(2).click();
  await showTab(page, "Properties");
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("15");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(3).click();
  await showTab(page, "Properties");
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("15");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
});

test("distribute spaces the pieces inside a border selected with them", async ({ page }) => {
  await seedAlignExtras(page);
  await showTab(page, "Properties");
  const horizontal = page.getByRole("button", { name: "Distribute horizontal spacing" });

  // Widen red to 100 mm, a border across x over green (0..10) and the Group (50..80).
  await selectRows(page, [0]);
  await showTab(page, "Properties");
  await page.getByLabel("W", { exact: true }).fill("100");
  await expect.poll(async () => (await nodeTransform(page, 2))[0]).toBeCloseTo(10, 6);

  await selectRows(page, [0, 2]);
  await showTab(page, "Properties");
  await expect(horizontal).toBeDisabled();
  await expect(horizontal).toHaveAttribute("title", /select three or more pieces/);

  // 100 mm less 40 mm of pieces leaves 60 over three spaces: green to 20, the Group already at 50.
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(1).click({ modifiers: ["Shift"] });
  await showTab(page, "Properties");
  await horizontal.click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(2); // the W edit, then this
  expect((await commitLog(page))[1]).toMatchObject({ ids: [3], m: [1, 0, 0, 1, 20, 0] });
});

test("a border too small for its pieces blocks distribute on that axis only, and says why", async ({ page }) => {
  await seedAlignExtras(page);
  await showTab(page, "Properties");
  const horizontal = page.getByRole("button", { name: "Distribute horizontal spacing" });
  await showTab(page, "Properties");
  const vertical = page.getByRole("button", { name: "Distribute vertical spacing" });

  // Red becomes a 100 mm border; green grows to 75 mm, so with the 30 mm Group the pieces need
  // 105 mm across. Down y nothing spans and the three distribute between two ends as usual.
  await selectRows(page, [0]);
  await showTab(page, "Properties");
  await page.getByLabel("W", { exact: true }).fill("100");
  await expect.poll(async () => (await nodeTransform(page, 2))[0]).toBeCloseTo(10, 6);
  await selectRows(page, [1]);
  await showTab(page, "Properties");
  await page.getByLabel("W", { exact: true }).fill("75");
  await expect.poll(async () => (await nodeTransform(page, 3))[0]).toBeCloseTo(7.5, 6);

  await selectRows(page, [0, 1, 2]);
  await showTab(page, "Properties");
  await expect(horizontal).toBeDisabled();
  await expect(horizontal).toHaveAttribute("title", /the pieces do not fit inside the one around them/);
  await expect(vertical).toBeEnabled();
  await expect(vertical).toHaveAttribute("title", "Distribute vertical spacing");
});

// #298: a commit drained from the queue was made before the refusal ahead of it arrived, so it
// must not clear that refusal's message before the operator has seen it.
test("a refusal stays on screen when an edit queued behind it lands", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => {
    const w = window as unknown as { __holdCommits: () => void; __failNextCommit: () => void };
    w.__holdCommits();
    w.__failNextCommit();
  });
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // will be refused
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("30"); // queued behind it
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(30, 6);
  await expect(page.getByText("transform refused")).toBeVisible();
});

test("a queued edit whose refresh succeeds clears the refresh warning ahead of it", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // lands, refresh fails
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("30"); // queued behind it
  await page.evaluate(() => {
    const w = window as unknown as { __failNextSnapshot: () => void; __releaseCommits: () => Promise<void> };
    w.__failNextSnapshot();
    return w.__releaseCommits();
  });

  // The warning was true when it went up; once the queued edit has re-read the canvas it is not.
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(30, 6);
  await expect(page.getByText("the canvas could not be refreshed")).toBeHidden();
});

test("a fresh edit after a refusal clears its message", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __failNextCommit: () => void }).__failNextCommit());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0);
  await expect(page.getByText("transform refused")).toBeVisible();

  // Made after the refusal was on screen, so it is the operator's next act and starts clean.
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("30");
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(30, 6);
  await expect(page.getByText("transform refused")).toBeHidden();
});

// Copilot on #301: going out from the queue is not the same as being made before the refusal. An
// align clicked once the refusal is on screen, while an edit queued earlier is still on the wire,
// is the operator's next act and must clear it when it lands.
test("an edit made after a refusal was shown clears it, even when it waits in the queue", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => {
    const w = window as unknown as { __holdCommits: () => void; __failNextCommit: () => void };
    w.__holdCommits();
    w.__failNextCommit();
  });
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // will be refused
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("30"); // queued before the refusal
  // Let the refusal through and hold again at once, so the X edit it drains stays on the wire.
  await page.evaluate(() => {
    const w = window as unknown as { __holdCommits: () => void; __releaseCommits: () => Promise<void> };
    void w.__releaseCommits();
    w.__holdCommits();
  });
  await expect(page.getByText("transform refused")).toBeVisible();

  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align horizontal centres" }).click(); // queued behind X
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(160, 6);
  await expect(page.getByText("transform refused")).toBeHidden();
});

test("a newer align replaces a queued one on the same selection in any click order", async ({ page }) => {
  // Copilot on #301: the key followed click order, so reselecting a piece made a second request
  // that ran after the first instead of replacing it.
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] }); // [red, Group]
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click();
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(2).click({ modifiers: ["Shift"] }); // [Group]
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(2).click({ modifiers: ["Shift"] }); // [Group, red]
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align right edges" }).click();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // Right alone: red to the Group's 40 (+25). Left first would have moved the Group to 5.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1]).toMatchObject({ ids: [4], m: [1, 0, 0, 1, expect.closeTo(25, 6), 0] });
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 30, 0]);
});

test("a queued align to the artboard uses the bed it lands on, not the one it was clicked on", async ({ page }) => {
  // Copilot on #301: the queued click kept the render's artboard, so a machine switch in between
  // centred the piece on the old bed.
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align horizontal centres" }).click(); // queued

  const before = (await readView(page)).scale;
  await showTab(page, "Properties");
  await page.getByLabel("Design for").selectOption("puma"); // 600 mm bed
  await expect.poll(async () => (await readView(page)).scale).not.toBeCloseTo(before, 6);
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // Red's centre is 10; the Puma's is 300. The Cameo's 165 would give 155.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1]).toMatchObject({ ids: [4], m: [1, 0, 0, 1, expect.closeTo(290, 6), 0] });
});

test("the align row follows the preview while its commit is on the wire", async ({ page }) => {
  // Copilot on #301: the preview lived in a ref, so the buttons kept the bounds from before it.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedGroup: true });
  await page.goto("/");
  await selectRows(page, [0, 2, 3]); // the Group, red, green: all 10 mm wide
  await showTab(page, "Properties");
  const horizontal = page.getByRole("button", { name: "Distribute horizontal spacing" });
  await expect(horizontal).toBeEnabled();

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click();
  // In the preview all three sit at 0..10, so each spans the rest and distribute cannot act.
  await expect(horizontal).toBeDisabled();
  await expect(horizontal).toHaveAttribute("title", /spans the selection on this axis/);
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());
});

test("the fake refuses a whole batch when a later entry's geometry cannot be reversed", async ({ page }) => {
  // Copilot on #301: the first entry is fine and the second sits under a Group scaled to nothing.
  // Rust refuses the batch and keeps neither; the fake must too, or a test can pass on a half-commit.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedCollapsedGroup: true });
  await page.goto("/");
  const refused = await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("commit_transforms", { moves: [
        { ids: [2], m: [1, 0, 0, 1, 5, 0] },
        { ids: [5], m: [1, 0, 0, 1, 1, 0] },
      ] }).then(() => false, () => true),
  );
  expect(refused).toBe(true);
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 0, 0]);
  expect(await nodeTransform(page, 5)).toEqual([1, 0, 0, 1, 0, 0]);
  // Nor logged: a refused batch that took a batch number would shift every later one.
  expect(await commitLog(page)).toEqual([]);
});

test("the fake accepts a batch under a tiny but invertible parent, as Rust does", async ({ page }) => {
  // Copilot on #301: Rust refuses only an exactly zero determinant, so a 1e-7 scale (determinant
  // 1e-14) must commit; a tolerance in the fake refused it and could fail a valid test.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedCollapsedGroup: true, collapsedScale: 1e-7 });
  await page.goto("/");
  const refused = await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("commit_transforms", { moves: [
        { ids: [2], m: [1, 0, 0, 1, 5, 0] },
        { ids: [5], m: [1, 0, 0, 1, 1, 0] },
      ] }).then(() => false, () => true),
  );
  expect(refused).toBe(false);
  expect((await nodeTransform(page, 2))[4]).toBeCloseTo(5, 6);
  // A 1 mm world move beneath a 1e-7 scale is 1e7 in the rect's own space (CodeRabbit on #301).
  expect((await nodeTransform(page, 5))[4] / 1e7).toBeCloseTo(1, 6);
});

test("the fake refuses an empty batch, or a batch with an empty entry, as transform_each does", async ({ page }) => {
  // Copilot on #301: Rust answers EmptySelection for both and keeps nothing; the fake applied the
  // valid first move and recorded the batch.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const send = (moves: unknown[]) => page.evaluate((mv) =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("commit_transforms", { moves: mv }).then(() => false, () => true), moves);
  expect(await send([{ ids: [2], m: [1, 0, 0, 1, 5, 0] }, { ids: [], m: [1, 0, 0, 1, 1, 0] }])).toBe(true);
  expect(await send([])).toBe(true);
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 0, 0]);
  expect(await commitLog(page)).toEqual([]);
});

test("a queued edit is dropped when Reload replaces the document", async ({ page }) => {
  // Copilot on #301: Reload reuses ids, so an X edit queued against the old document moved a shape
  // in the reloaded one.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save"); // gives Reload a path
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // on the wire
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("30"); // queued behind it
  await fileAction(page, "Revert to saved");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row").first()).toBeVisible();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The queued X must not follow the held drag into the reloaded document. Where the drag itself
  // ends up is not this test's to say: the backend runs it before the load, which then replaces it,
  // while the fake runs a held commit on release, after the load.
  await page.waitForTimeout(300);
  expect(await commitLog(page)).toHaveLength(1);
  expect((await nodeTransform(page, 2))[4]).not.toBeCloseTo(30, 6);
});

test("an edit made while Reload loads is held, and dropped once the document is replaced", async ({ page }) => {
  // An edit made during the load used to go straight out, with ids from the document on screen, and
  // land on whatever the reloaded one gives those ids.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save"); // gives Reload a path
  await page.evaluate(() => (window as unknown as { __holdLoad: () => void }).__holdLoad());
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red
  await fileAction(page, "Revert to saved"); // parked
  // Locked while it loads, and saying why: the panel used to take these, and the load then dropped
  // them without a word (Copilot on #301).
  await showTab(page, "Properties");
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await expect(centre).toBeDisabled();
  await expect(centre).toHaveAttribute("title", /waiting for the document to load/);
  await showTab(page, "Properties");
  await expect(page.getByLabel("X", { exact: true })).toBeDisabled();
  await page.evaluate(() => (window as unknown as { __releaseLoad: () => Promise<void> }).__releaseLoad());

  await page.waitForTimeout(300);
  expect(await commitLog(page)).toEqual([]);
  expect(await nodeTransform(page, 2)).toEqual([1, 0, 0, 1, 0, 0]);
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await expect(centre).toBeEnabled();
  await expect(page.getByLabel("X", { exact: true })).toBeEnabled();
});

test("a snapshot answering after a newer one has rendered is dropped", async ({ page }) => {
  // CodeRabbit on #301: an older snapshot answering last rendered under a newer revision, so the
  // canvas went back to geometry from before the align that the newer one already showed.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red
  await showTab(page, "Properties");
  const x = page.getByLabel("X", { exact: true });
  const before = await x.inputValue();
  await page.evaluate(() => (window as unknown as { __holdSnapshots: () => void }).__holdSnapshots());
  await page.getByRole("checkbox", { name: "Cut this shape" }).click(); // its snapshot is held
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align horizontal centres" }).click(); // to the bed; held too
  await expect.poll(() => commitLog(page).then((l) => l.length)).toBe(1);

  await page.evaluate(() => (window as unknown as { __releaseLatestSnapshot: () => Promise<void> }).__releaseLatestSnapshot());
  await expect(x).not.toHaveValue(before);
  const aligned = await x.inputValue();
  await page.evaluate(() => (window as unknown as { __releaseSnapshot: () => Promise<void> }).__releaseSnapshot());
  await page.waitForTimeout(300);
  await expect(x).toHaveValue(aligned);
});

test("Open or Reload waits for a document edit already on its way before it loads", async ({ page }) => {
  // CodeRabbit on #301: the lock is checked when an edit starts, and an import then reads its file
  // before it sends. A Reload in that gap loaded first, and the import's old root met the new
  // document. A load now waits for edits already started, as it does for a transform commit.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  const loads = () => page.evaluate(() => (window as unknown as { __loads?: number }).__loads ?? 0);
  await page.evaluate(() => (window as unknown as { __holdEdits: () => void }).__holdEdits());
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red
  await showTab(page, "Properties");
  await page.getByRole("checkbox", { name: "Cut this shape" }).click(); // on its way, held
  await fileAction(page, "Revert to saved");
  await page.waitForTimeout(300);
  expect(await loads()).toBe(0);
  await page.evaluate(() => (window as unknown as { __releaseEdits: () => Promise<void> }).__releaseEdits());

  await expect.poll(loads).toBe(1);
  expect(await page.evaluate(() => (window as unknown as { __loadDuringEdit?: boolean }).__loadDuringEdit)).toBeUndefined();
});

test("an import still reading its file finishes before Reload loads", async ({ page }) => {
  // CodeRabbit on #301: the import passes the lock, then awaits its file before it sends. Paused in
  // that read, a Reload must wait for it rather than load and take the import's old root.
  await page.addInitScript(() => {
    const w = window as unknown as {
      __docOrder: string[]; __holdFileReads: () => void; __releaseFileReads: () => void; __heldFileReads: () => number;
    };
    w.__docOrder = [];
    let holding = false;
    const held: (() => void)[] = [];
    const read = Blob.prototype.arrayBuffer;
    Blob.prototype.arrayBuffer = function (this: Blob) {
      if (!holding) return read.call(this);
      return new Promise((resolve, reject) => held.push(() => read.call(this).then(resolve, reject)));
    };
    w.__holdFileReads = () => { holding = true; };
    w.__releaseFileReads = () => { holding = false; held.splice(0).forEach((f) => f()); };
    w.__heldFileReads = () => held.length;
  });
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  await page.evaluate(() => (window as unknown as { __holdFileReads: () => void }).__holdFileReads());
  // Built in the page: the e2e build has no Node types, so no Buffer.
  await page.evaluate(() => {
    const input = document.querySelector<HTMLInputElement>('input[type="file"]')!;
    const files = new DataTransfer();
    files.items.add(new File(['<svg xmlns="http://www.w3.org/2000/svg"><rect width="5" height="5"/></svg>'], "a.svg", { type: "image/svg+xml" }));
    input.files = files.files;
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
  // In the read, past the lock check, before Reload is asked for.
  await expect.poll(() => page.evaluate(() => (window as unknown as { __heldFileReads: () => number }).__heldFileReads())).toBe(1);
  await fileAction(page, "Revert to saved");
  await page.waitForTimeout(300);
  const order = () => page.evaluate(() => (window as unknown as { __docOrder: string[] }).__docOrder);
  expect(await order()).toEqual([]);
  await page.evaluate(() => (window as unknown as { __releaseFileReads: () => void }).__releaseFileReads());

  await expect.poll(order).toEqual(["import_svg", "load_project"]);
});

test("a transform's late snapshot retires its preview against the newer one already shown", async ({ page }) => {
  // Copilot on #301: an align's snapshot answering after a Delete's had rendered was dropped as
  // "nothing rendered", so the align's preview stayed up over the newer scene, still drawing (and
  // hit-testing) the shape the Delete had removed.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const v = await fittedView(page);
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red
  await showTab(page, "Properties");
  const x = page.getByLabel("X", { exact: true });
  await page.evaluate(() => (window as unknown as { __holdSnapshots: () => void }).__holdSnapshots());
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align horizontal centres" }).click(); // its snapshot is held
  await expect.poll(() => commitLog(page).then((l) => l.length)).toBe(1);
  await expect(x).not.toHaveValue("0");
  const alignedX = Number(await x.inputValue());
  await page.keyboard.press("Delete"); // its snapshot is held too

  await page.evaluate(() => (window as unknown as { __releaseLatestSnapshot: () => Promise<void> }).__releaseLatestSnapshot());
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(1); // the Delete's, red gone
  await page.evaluate(() => (window as unknown as { __releaseSnapshot: () => Promise<void> }).__releaseSnapshot());
  await page.waitForTimeout(300);

  const at = await toPage(page, v, { x: alignedX + 5, y: 5 });
  await page.mouse.click(at.x, at.y);
  await expect(x).toHaveCount(0);
});

test("Delete, Undo and the other document commands are refused while a document loads", async ({ page }) => {
  // CodeRabbit on #301: only transforms waited for a load. A Delete pressed during a Reload named ids
  // from the document on screen, and the loaded one reuses them.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  await page.evaluate(() => (window as unknown as { __holdLoad: () => void }).__holdLoad());
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red
  await fileAction(page, "Revert to saved"); // held
  await page.keyboard.press("Delete");
  await expect(page.getByText("the document is still loading")).toBeVisible();
  await page.evaluate(() => (window as unknown as { __releaseLoad: () => Promise<void> }).__releaseLoad());

  await page.waitForTimeout(300);
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(2);
});

test("an edit refused after a load whose read failed says so, reads it again, and unlocks", async ({ page }) => {
  // After Reload loaded but its snapshot failed, every edit was refused as "still loading" until
  // another Reload, which nothing said to do (silent-failure-hunter on #301).
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  const failNextSnapshot = () =>
    page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await failNextSnapshot();
  await fileAction(page, "Revert to saved"); // loads; its read fails
  await expect(page.getByText("snapshot unavailable")).toBeVisible();

  await showTab(page, "Properties");
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await expect(centre).toBeDisabled();
  await expect(centre).toHaveAttribute("title", /the loaded document could not be read\. Reload to try again/);

  await failNextSnapshot(); // the re-read fails too
  await page.keyboard.press("Delete");
  await expect(page.getByText(/Not applied: the loaded document could not be read: snapshot unavailable\. Reload to try again/)).toBeVisible();
  await expect(centre).toBeDisabled();

  await page.keyboard.press("Delete"); // this re-read works
  await expect(page.getByText(/Not applied, but the loaded document is on screen now/)).toBeVisible();
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row")).toHaveCount(2); // neither Delete went out
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await expect(centre).toBeEnabled();
});

test("a snapshot asked for before a later load never renders over it or lifts its lock", async ({ page }) => {
  // Copilot on #301: the lock lifted on any newer snapshot, so a first Reload's snapshot answering
  // after a second Reload had loaded showed the first document and unlocked edits on it, while the
  // backend held the second; an align there sent the first one's bounds into the second.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  const loads = () => page.evaluate(() => (window as unknown as { __loads?: number }).__loads ?? 0);
  await page.evaluate(() => (window as unknown as { __holdSnapshots: () => void }).__holdSnapshots());
  await fileAction(page, "Revert to saved"); // loads; its snapshot is held
  await expect.poll(loads).toBe(1);
  await fileAction(page, "Revert to saved"); // loads again; held too
  await expect.poll(loads).toBe(2);

  const release = () =>
    page.evaluate(() => (window as unknown as { __releaseSnapshot: () => Promise<void> }).__releaseSnapshot());
  await showTab(page, "Properties");
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await release(); // the first Reload's answer, read before the second load
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await expect(centre).toHaveAttribute("title", /waiting for the document to load/);
  await expect(centre).toBeDisabled();

  await release(); // the second's, read after its load
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await expect(centre).toBeEnabled();
});

test("a queued align finds a Group's shapes in the tree as it stands when it is sent", async ({ page }) => {
  // Copilot on #301: the queued click kept the tree from its render, so a shape added to the Group
  // before it drained was left out of the Group's bounds.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedAlignExtras: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red, 0..10
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(2).click({ modifiers: ["Shift"] }); // + the Group, 50..80
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click(); // queued
  // A 10 mm rect joins the Group at 0..10 while the align waits.
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("add_primitive", { parent: 4, kind: { Rect: { w: 10, h: 10 } } }));
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // With the new rect the Group reaches 0, so only red moves (to 0). The old tree said 50..80,
  // which would have sent the Group to 5 instead.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  expect((await commitLog(page))[1]).toMatchObject({ ids: [2], m: [1, 0, 0, 1, expect.closeTo(-5, 6), 0] });
});

test("an edit made between a commit settling and the queue draining still waits its turn", async ({ page }) => {
  // CodeRabbit on #301: inFlight cleared on settle but the queue drained a render later, so an
  // edit made in between was sent at once and overlapped the drained one on the wire.
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 5 * v.scale); // red to 5..15 × 5..15
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] }); // + the Group
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align top edges" }).click(); // queued behind the drag

  // Release without yielding a task, let the settled chain run its microtasks, then click: React
  // has not rendered yet, so this lands in the gap before the drain.
  await page.evaluate(async () => {
    void (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits();
    for (let i = 0; i < 50; i++) await Promise.resolve();
    (document.querySelector('[aria-label="Align right edges"]') as HTMLButtonElement).click();
  });

  await expect.poll(async () => (await commitLog(page)).length).toBe(3);
  expect(await page.evaluate(() => (window as unknown as { __maxInFlightCommits?: number }).__maxInFlightCommits)).toBe(1);
  // In the order they were made: top was queued first. One on the wire at a time is not enough on
  // its own; a click in the gap that went out at once would still land before the queued top.
  const [, top, right] = await commitLog(page);
  expect(top).toMatchObject({ ids: [4], m: [1, 0, 0, 1, 0, expect.closeTo(-5, 6)] });
  expect(right).toMatchObject({ ids: [4], m: [1, 0, 0, 1, expect.closeTo(25, 6), 0] });
});

test("Reload waits for a commit already on the wire before it loads", async ({ page }) => {
  // CodeRabbit on #301: gating new edits does not cover a commit that already went out. Open and
  // Reload must not start load_project while one is unanswered, or a reused id could meet it.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // on the wire, held
  await fileAction(page, "Revert to saved");
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The load ran (a never-run Reload would pass a no-overlap check), and did not overlap the commit.
  await expect.poll(() => page.evaluate(() => (window as unknown as { __loads?: number }).__loads ?? 0)).toBe(1);
  expect(await page.evaluate(() => (window as unknown as { __loadDuringCommit?: boolean }).__loadDuringCommit ?? false)).toBe(false);
  // The reload did happen once the commit had settled: the saved copy has red back at 0.
  await expect.poll(async () => (await nodeTransform(page, 2))[4]).toBeCloseTo(0, 6);
});

test("after a load whose snapshot failed, edits on the old view are not sent", async ({ page }) => {
  // Copilot on #301: replacing cleared when the load returned, but the new document's snapshot
  // comes after, so the canvas still showed the old one; an align there sent old bounds and a
  // reused id into the loaded document. Edits stay off until a newer snapshot has rendered.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  await page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await fileAction(page, "Revert to saved");
  await expect(page.getByText("snapshot unavailable")).toBeVisible(); // loaded, but not re-read

  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red, from the old view
  // Locked, and saying why, rather than taking the click and dropping it (silent-failure-hunter).
  await showTab(page, "Properties");
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await expect(centre).toBeDisabled();
  await expect(centre).toHaveAttribute("title", /the loaded document could not be read/);
  await showTab(page, "Properties");
  await expect(page.getByLabel("X", { exact: true })).toBeDisabled();
  expect(await commitLog(page)).toEqual([]);
});

test("a drag held through a Reload is dropped, not sent to the loaded document", async ({ page }) => {
  // Copilot on #301: an unreleased drag is not a commit yet, so the load did not wait for it, and
  // its pointer-up sent the old ids and matrix straight into the reloaded document.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  const from = await toPage(page, v, { x: 5, y: 5 });
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 5 * v.scale, from.y, { steps: 4 }); // dragging, not released
  await page.getByRole("button", { name: "File", exact: true }).focus();
  await page.keyboard.press("Enter");
  await page.getByRole("menuitem", { name: "Revert to saved" }).focus();
  await page.keyboard.press("Enter");
  await showTab(page, "Layers");
  await expect(page.getByTestId("layer-row").first()).toBeVisible();
  await page.mouse.up();

  await page.waitForTimeout(300);
  expect(await commitLog(page)).toEqual([]);
  expect((await nodeTransform(page, 2))[4]).toBeCloseTo(0, 6);
});

test("the fake refuses a held batch whose node was deleted before it was answered", async ({ page }) => {
  // Copilot on #301: the id check ran at the call and the staged apply skipped a node gone by
  // the answer, so the fake applied the rest where Rust refuses the batch.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const refused = await page.evaluate(async () => {
    const w = window as unknown as {
      __holdCommits: () => void; __releaseCommits: () => Promise<void>;
      __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> };
    };
    w.__holdCommits();
    const batch = w.__TAURI_INTERNALS__.invoke("commit_transforms", { moves: [
      { ids: [2], m: [1, 0, 0, 1, 5, 0] },
      { ids: [3], m: [1, 0, 0, 1, 5, 0] },
    ] }).then(() => false, () => true);
    await w.__TAURI_INTERNALS__.invoke("delete", { ids: [2] });
    await w.__releaseCommits();
    return batch;
  });
  expect(refused).toBe(true);
  expect(await nodeTransform(page, 3)).toEqual([1, 0, 0, 1, 0, 0]);
});

test("edits work again once a successful Reload's snapshot has rendered", async ({ page }) => {
  // pr-test-analyzer on #301: the lock after a load was tested only while held. Were its release
  // broken, every edit after any Open or Reload would be dead and nothing else would notice.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  await fileAction(page, "Revert to saved");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red
  await showTab(page, "Properties");
  const centre = page.getByRole("button", { name: "Align horizontal centres" });
  await expect(centre).toBeEnabled();
  await centre.click();
  await expect.poll(async () => (await commitLog(page)).length).toBe(1);
  expect((await commitLog(page))[0]).toMatchObject({ ids: [2], m: [1, 0, 0, 1, expect.closeTo(160, 6), 0] });
});

test("a queued align on two pieces sends nothing if one is deleted before it goes", async ({ page }) => {
  // pr-test-analyzer on #301: an align clicked on two pieces must not turn into "send the survivor
  // to the mat" because the other went while it waited.
  const v = await selectRedBesideGroup(page);
  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // red to 5..15, parked
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click({ modifiers: ["Shift"] }); // + the Group
  await showTab(page, "Properties");
  await page.getByRole("button", { name: "Align left edges" }).click(); // queued on [red, Group]
  await page.evaluate(() =>
    (window as unknown as { __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> } })
      .__TAURI_INTERNALS__.invoke("delete", { ids: [2] })); // the Group goes
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The drag lands and its refresh drops the Group; the align, now on red alone, sends nothing.
  await expect.poll(async () => (await nodeTransform(page, 4))[4]).toBeCloseTo(5, 6);
  await page.waitForTimeout(300);
  expect(await commitLog(page)).toHaveLength(1);
});

test("a refused edit puts back the preview of an unread commit beneath it", async ({ page }) => {
  // pr-test-analyzer on #301: every refusal test started with no preview standing. Here the first
  // drag lands but cannot be re-read, so its preview stands for what the backend holds; a refused
  // edit on top must return to that, not to the stale committed scene.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);

  await page.evaluate(() => (window as unknown as { __failNextSnapshot: () => void }).__failNextSnapshot());
  await dragBy(page, await toPage(page, v, { x: 5, y: 5 }), 5 * v.scale, 0); // lands at 5, unread
  await expect(page.getByText("the canvas could not be refreshed")).toBeVisible();
  await showTab(page, "Properties");
  // A drag of exactly 5 mm lands on a float a couple of ulps off once the canvas is not the
  // whole window; the field shows that number, and "5" is what it means.
  await expect.poll(async () => Number(await page.getByLabel("X", { exact: true }).inputValue())).toBeCloseTo(5, 5);

  await page.evaluate(() => (window as unknown as { __failNextCommit: () => void }).__failNextCommit());
  await showTab(page, "Properties");
  await page.getByLabel("X", { exact: true }).fill("30");
  await expect(page.getByText("transform refused")).toBeVisible();
  await showTab(page, "Properties");
  await expect.poll(async () => Number(await page.getByLabel("X", { exact: true }).inputValue())).toBeCloseTo(5, 5);
});

test("an align pressed from the keyboard during a drag waits for the drag's commit", async ({ page }) => {
  // Copilot and CodeRabbit on #301: busy() ignored a drag under the pointer, so the align went out
  // at once and the release sent a second batch on top of it.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click(); // red
  const v = await zoomInAt(page, { x: 5, y: 5 }, -350);
  // Properties has to be showing before the pointer goes down: clicking the tab mid-drag
  // is a click the canvas still owns.
  await showTab(page, "Properties");

  await page.evaluate(() => (window as unknown as { __holdCommits: () => void }).__holdCommits());
  const from = await toPage(page, v, { x: 5, y: 5 });
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 5 * v.scale, from.y, { steps: 4 }); // dragging
  await page.getByRole("button", { name: "Align horizontal centres" }).focus();
  await page.keyboard.press("Enter");
  await page.mouse.up();
  await page.evaluate(() => (window as unknown as { __releaseCommits: () => Promise<void> }).__releaseCommits());

  // The drag first, then the align computed from where it left red; never two on the wire.
  await expect.poll(async () => (await commitLog(page)).length).toBe(2);
  const [drag, align] = await commitLog(page);
  expect(drag.m[4]).toBeCloseTo(5, 6);
  expect(align.m[4]).toBeCloseTo(165 - 10, 6);
  expect(await page.evaluate(() => (window as unknown as { __maxInFlightCommits?: number }).__maxInFlightCommits)).toBe(1);
});

test("a second Open or Reload while one is still loading is refused, and edits stay held", async ({ page }) => {
  // Copilot on #301: two replacements could overlap, and the first to finish cleared the shared
  // lock while the other was still loading, letting old-document edits reach it.
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await fileAction(page, "Save");
  await page.evaluate(() => (window as unknown as { __holdLoad: () => void }).__holdLoad());
  await fileAction(page, "Revert to saved"); // held
  await fileAction(page, "Revert to saved"); // refused while the first loads
  await expect(page.getByText("another document is still loading")).toBeVisible();
  await page.evaluate(() => (window as unknown as { __releaseLoad: () => Promise<void> }).__releaseLoad());
  await expect.poll(() => page.evaluate(() => (window as unknown as { __loads?: number }).__loads ?? 0)).toBe(1);

  // Once the first load's snapshot renders, editing works.
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await expect(page.getByRole("button", { name: "Align horizontal centres" })).toBeEnabled();
});

test("appearance is remembered across a reload", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  const appearance = page.getByRole("button", { name: "Appearance" });
  await expect(appearance).toBeEnabled();
  await appearance.click();
  await page.keyboard.press("Escape");
  await expect(appearance).toBeFocused();
  await expect(page.getByRole("menu", { name: "Appearance" })).toHaveCount(0);

  await appearance.click();
  await page.getByRole("menuitemradio", { name: "Dark" }).click();
  await expect(appearance).toBeFocused();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("cuthulhu.theme"))).toBe("dark");

  await page.reload();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.getByRole("button", { name: "Appearance" }).click();
  await expect(page.getByRole("menuitemradio", { name: "Dark" })).toHaveAttribute("aria-checked", "true");

  await page.getByRole("menuitemradio", { name: "Light" }).click();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await page.reload();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await page.getByRole("button", { name: "Appearance" }).click();
  await expect(page.getByRole("menuitemradio", { name: "Light" })).toHaveAttribute("aria-checked", "true");
});

test("system appearance follows the operating system and an explicit one does not", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Appearance" })).toBeEnabled();
  await expect(page.locator("html")).not.toHaveClass(/dark/);

  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).not.toHaveClass(/dark/);

  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveClass(/dark/);

  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Light" }).click();
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).not.toHaveClass(/dark/);
});

test("the boot script paints the cached theme before preferences answer", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.addInitScript(() => localStorage.setItem("cuthulhu.theme", "dark"));
  await page.addInitScript(installMockTauri, { holdPreferences: true });
  await page.goto("/");
  await expect(page.locator("html")).toHaveClass(/dark/);
  await expect.poll(() =>
    page.evaluate(() => typeof (window as unknown as { __releasePreferences?: unknown }).__releasePreferences === "function"),
  ).toBe(true);
  await expect(page.locator("html")).toHaveClass(/dark/);
  await page.evaluate(() => (window as unknown as { __releasePreferences: () => void }).__releasePreferences());
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await expect(page.getByRole("button", { name: "Appearance" })).toBeEnabled();
});

async function paintContrast(page: Page, locator: Locator) {
  const bytes = (await locator.screenshot()) as unknown as Uint8Array;
  return contrastFromPng(page, bytes);
}

async function contrastFromPng(page: Page, bytes: Uint8Array) {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  const rgba = await page.evaluate(async (png) => {
    const img = new Image();
    img.src = `data:image/png;base64,${png}`;
    await img.decode();
    const canvas = document.createElement("canvas");
    canvas.width = img.naturalWidth;
    canvas.height = img.naturalHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("no canvas");
    ctx.drawImage(img, 0, 0);
    return Array.from(ctx.getImageData(0, 0, canvas.width, canvas.height).data);
  }, btoa(binary));
  return renderedContrastFromPixels(rgba);
}

test("top-bar labels stay readable in both themes, including after a light-to-dark switch", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  const importButton = page.getByRole("button", { name: "Import", exact: true });
  const fileButton = page.getByRole("button", { name: "File", exact: true });
  const undo = page.getByRole("button", { name: "Undo", exact: true });
  const redo = page.getByRole("button", { name: "Redo", exact: true });
  const layers = page.getByRole("tab", { name: "Layers", exact: true });
  const properties = page.getByRole("tab", { name: "Properties", exact: true });
  const devices = page.getByRole("tab", { name: "Devices", exact: true });
  await expect(importButton).toBeEnabled();
  await expect(undo).toBeDisabled();
  await expect(page.locator("html")).not.toHaveClass(/dark/);

  const measure = async () => ({
    import: await paintContrast(page, importButton),
    file: await paintContrast(page, fileButton),
    undo: await paintContrast(page, undo),
    redo: await paintContrast(page, redo),
    layers: await paintContrast(page, layers),
    properties: await paintContrast(page, properties),
    devices: await paintContrast(page, devices),
  });

  const light = await measure();
  expect(await importButton.evaluate((el) => getComputedStyle(el).color)).toBe("rgb(1, 47, 54)");
  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Dark" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);
  // `transition-colors` holds the light foreground for a frame after the class
  // flips. The settled button color is the dark token.
  await expect.poll(() => importButton.evaluate((el) => getComputedStyle(el).color)).toBe("rgb(230, 240, 240)");
  await expect.poll(() => properties.evaluate((el) => getComputedStyle(el).color)).toBe("rgb(157, 184, 185)");
  // Active tab: the light theme's dark teal on a white pill, in both themes.
  await expect.poll(() => layers.evaluate((el) => getComputedStyle(el).color)).toBe("rgb(16, 108, 114)");
  await expect.poll(() => layers.evaluate((el) => getComputedStyle(el).backgroundColor)).toBe("rgb(255, 255, 255)");
  await layers.hover();
  await expect.poll(() => layers.evaluate((el) => getComputedStyle(el).backgroundColor)).toBe("rgb(255, 255, 255)");
  await layers.focus();
  await expect.poll(() => layers.evaluate((el) => getComputedStyle(el).color)).toBe("rgb(16, 108, 114)");
  const dark = await measure();

  for (const [theme, sample] of [
    ["light", light],
    ["dark", dark],
  ] as const) {
    // Printed so a failure names the pixels, and so the PR can quote the ratios.
    console.log(theme, {
      import: sample.import.ratio.toFixed(2),
      file: sample.file.ratio.toFixed(2),
      undo: sample.undo.ratio.toFixed(2),
      redo: sample.redo.ratio.toFixed(2),
      layers: sample.layers.ratio.toFixed(2),
      properties: sample.properties.ratio.toFixed(2),
      devices: sample.devices.ratio.toFixed(2),
      importInk: sample.import.ink.join(","),
      undoInk: sample.undo.ink.join(","),
      propertiesInk: sample.properties.ink.join(","),
      importPeak: sample.import.peak.toFixed(2),
      undoPeak: sample.undo.peak.toFixed(2),
    });
    expect(sample.import.ratio, `${theme} Import`).toBeGreaterThanOrEqual(4.5);
    expect(sample.file.ratio, `${theme} File`).toBeGreaterThanOrEqual(4.5);
    expect(sample.undo.ratio, `${theme} Undo`).toBeGreaterThanOrEqual(3);
    expect(sample.redo.ratio, `${theme} Redo`).toBeGreaterThanOrEqual(3);
    expect(sample.layers.ratio, `${theme} Layers`).toBeGreaterThanOrEqual(4.5);
    expect(sample.properties.ratio, `${theme} Properties`).toBeGreaterThanOrEqual(4.5);
    expect(sample.devices.ratio, `${theme} Devices`).toBeGreaterThanOrEqual(4.5);
  }
});

test("the File menu ring follows the keyboard, not a pointer click", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  const file = page.getByRole("button", { name: "File", exact: true });
  await page.getByRole("button", { name: "Import", exact: true }).focus();
  await page.keyboard.press("Tab");
  await expect(file).toBeFocused();
  await expect.poll(() => file.evaluate((el) => getComputedStyle(el).boxShadow)).not.toBe("none");

  await file.click();
  const menu = page.getByRole("menu");
  await expect(menu.getByRole("menuitem", { name: "Save", exact: true })).toBeVisible();
  // Radix hides the rest of the page from the accessibility tree while the menu is open.
  const trigger = page.locator('header button[aria-label="File"]');
  expect(await trigger.evaluate((el) => getComputedStyle(el).boxShadow)).toBe("none");
  expect(await menu.evaluate((el) => getComputedStyle(el).boxShadow)).toBe("none");
});

test("Import names Image, with Opens Trace beside it", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Import", exact: true }).click();
  const item = page.getByRole("menuitem", { name: "Image" });
  await expect(item).toBeVisible();
  await expect(item).toContainText("Opens Trace");
  await expect(item).not.toContainText("Image…");
});

test("muted and faded button hovers still reach the foreground", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  const idle = page.getByRole("group", { name: "Units" }).locator("button[aria-pressed='false']");
  await idle.hover();
  const foreground = await page.evaluate(() => getComputedStyle(document.body).color);
  await expect.poll(() => idle.evaluate((el) => getComputedStyle(el).color)).toBe(foreground);

  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await goToSendStep(page, 2);
  const machine = page.getByRole("dialog", { name: "Send to cutter" }).getByRole("button", { name: "Machine", exact: true });
  const faded = await machine.evaluate((el) => getComputedStyle(el).color);
  expect(faded).not.toBe(foreground);
  await machine.hover();
  await expect.poll(() => machine.evaluate((el) => getComputedStyle(el).color)).toBe(foreground);
});

test("the Appearance glyph is a 16px icon", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  const box = await page.getByRole("button", { name: "Appearance" }).locator("svg").boundingBox();
  expect(box?.width).toBeGreaterThanOrEqual(15);
  expect(box?.width).toBeLessThanOrEqual(17);
  expect(box?.height).toBeGreaterThanOrEqual(15);
  expect(box?.height).toBeLessThanOrEqual(17);
});

test("a machine with no saved unit takes inches from en-US once and then keeps them", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "language", {
      configurable: true,
      get: () => sessionStorage.getItem("cuthulhu.testLocale") || "en-US",
    });
  });
  await page.addInitScript(installMockTauri, { unsetUnits: true });
  await page.goto("/");
  const inches = page.getByRole("button", { name: "in", exact: true });
  const millimetres = page.getByRole("button", { name: "mm", exact: true });
  await expect(inches).toHaveAttribute("aria-pressed", "true");

  await page.evaluate(() => sessionStorage.setItem("cuthulhu.testLocale", "fr-FR"));
  await page.reload();
  await expect(inches).toHaveAttribute("aria-pressed", "true");
  await expect(millimetres).toHaveAttribute("aria-pressed", "false");
});

test("a unit chosen while the region save is in flight is the one that sticks", async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "language", {
      configurable: true,
      get: () => "en-US",
    });
  });
  await page.addInitScript(installMockTauri, { unsetUnits: true, holdFirstUnitSave: true });
  await page.goto("/");
  await expect.poll(() =>
    page.evaluate(() => typeof (window as unknown as { __releaseUnitSave?: unknown }).__releaseUnitSave === "function"),
  ).toBe(true);
  await page.getByRole("button", { name: "mm", exact: true }).click();
  await page.evaluate(() => (window as unknown as { __releaseUnitSave: () => void }).__releaseUnitSave());
  await expect(page.getByRole("button", { name: "mm", exact: true })).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() =>
    page.evaluate(() => (JSON.parse(sessionStorage.getItem("cuthulhu.preferences") || "{}") as { units?: string }).units),
  ).toBe("mm");
});

test("aiming at a host keeps the cutter after the local manager reports Disconnected", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedBusyHost: true });
  await page.goto("/");
  await page.getByRole("button", { name: /No cutter connected/ }).click();
  await page.getByRole("menuitem", { name: /this computer/ }).first().click();
  await expect(page.getByRole("button", { name: /Active cutter/ })).toBeVisible();

  await page.getByRole("button", { name: /Active cutter/ }).click();
  await page.getByRole("menuitem", { name: /Workshop Pi/ }).click();
  await expect.poll(() =>
    page.evaluate(() => (window as unknown as { __hostReleaseEmitted?: boolean }).__hostReleaseEmitted === true),
  ).toBe(true);
  await expect(page.getByRole("button", { name: /Active cutter: Silhouette Cameo 5 Alpha/ })).toContainText("Workshop Pi");
});

test("a history reply from an earlier refresh does not rewind Undo", async ({ page }) => {
  await page.addInitScript(installMockTauri, { holdHistory: true });
  await page.goto("/");
  const waiters = () =>
    page.evaluate(() => ((window as unknown as { __historyWaiters?: unknown[] }).__historyWaiters ?? []).length);
  // StrictMode runs the mount refresh twice. Both are queued before the edit's.
  await expect.poll(waiters).toBe(2);
  await page.getByRole("button", { name: "Rectangle", exact: true }).click();
  await page.mouse.click(400, 300);
  await expect.poll(waiters).toBe(3);

  const releaseLast = (canUndo: boolean) =>
    page.evaluate((undo) => {
      (window as unknown as { __releaseLastHistory: (flags: { can_undo: boolean; can_redo: boolean }) => void }).__releaseLastHistory({
        can_undo: undo,
        can_redo: false,
      });
    }, canUndo);
  // The edit's reply is the newest. The two mount replies are older and must not land after it.
  await releaseLast(true);
  await expect(page.getByRole("button", { name: "Undo", exact: true })).toBeEnabled();
  await releaseLast(false);
  await releaseLast(false);
  await expect(page.getByRole("button", { name: "Undo", exact: true })).toBeEnabled();
});

test("a refused machine change does not rebuild the open plan", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, failSetMachine: true });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  // The passes are the second step, and that step asks for a cutter first. The plan itself is
  // requested as the sheet opens, which is the signal this test used the rows for.
  await expect.poll(() => page.evaluate(() => (window as unknown as { __planCalls?: number }).__planCalls ?? 0)).toBeGreaterThan(0);
  const before = await page.evaluate(() => (window as unknown as { __planCalls?: number }).__planCalls ?? 0);
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await expect(page.getByText("could not set the machine")).toBeVisible();
  expect(await page.evaluate(() => (window as unknown as { __planCalls?: number }).__planCalls ?? 0)).toBe(before);
});

test("Option+S subtracts when the produced character is not s", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(0).click();
  await page.getByTestId("layer-row").nth(1).click({ modifiers: ["Shift"] });
  await page.evaluate(() => {
    document.body.dispatchEvent(
      new KeyboardEvent("keydown", { key: "ß", code: "KeyS", metaKey: true, altKey: true, bubbles: true, cancelable: true }),
    );
  });
  await expect.poll(() => page.evaluate(() => (window as unknown as { __booleanOp?: string }).__booleanOp)).toBe("Subtract");
});

type InkBox = { name: string; box: { x: number; y: number; width: number; height: number } };

/** Boxes first, then one full-page shot. A later clip is a different paint: the
 *  screenshot walk captures the frame the theme click returns on. */
async function contrastsOnPageShot(
  page: Page,
  items: { name: string; locator: Locator; inset?: number }[],
) {
  const regions: InkBox[] = [];
  for (const item of items) {
    const box = await item.locator.boundingBox();
    if (!box) throw new Error(`no box for ${item.name}`);
    regions.push({ name: item.name, box: insetBox(box, item.inset ?? 0) });
  }
  const bytes = (await page.screenshot()) as unknown as Uint8Array;
  return contrastsInPng(page, regions, bytes);
}

/** The control's own fill, not the bar it sits on. A white pill's box includes
 *  the dark zoom bar at the edge, and that edge outranks the label. */
function insetBox(box: InkBox["box"], inset: number): InkBox["box"] {
  if (inset <= 0) return box;
  return {
    x: box.x + inset,
    y: box.y + inset,
    width: Math.max(1, box.width - inset * 2),
    height: Math.max(1, box.height - inset * 2),
  };
}

async function readPngRegions(page: Page, regions: InkBox[], bytes: Uint8Array) {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return page.evaluate(
    async (payload: { png: string; regions: InkBox[] }) => {
      const img = new Image();
      img.src = `data:image/png;base64,${payload.png}`;
      await img.decode();
      const canvas = document.createElement("canvas");
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("no canvas");
      ctx.drawImage(img, 0, 0);
      const scale = img.naturalWidth / window.innerWidth;
      return payload.regions.map((region) => {
        const width = Math.max(1, Math.round(region.box.width * scale));
        const height = Math.max(1, Math.round(region.box.height * scale));
        const data = ctx.getImageData(
          Math.round(region.box.x * scale),
          Math.round(region.box.y * scale),
          width,
          height,
        ).data;
        return { name: region.name, rgba: Array.from(data), width, height };
      });
    },
    { png: btoa(binary), regions },
  );
}

async function contrastsInPng(page: Page, regions: InkBox[], bytes: Uint8Array) {
  const crops = await readPngRegions(page, regions, bytes);
  return crops.map((crop) => ({ name: crop.name, hit: renderedContrastFromPixels(crop.rgba) }));
}

const JOB_INK = [
  { name: "Cancel job", role: "button" as const, min: 4.5 },
  { name: "Collapse", role: "button" as const, min: 4.5 },
  { name: "Import", role: "button" as const, min: 4.5 },
  { name: "File", role: "button" as const, min: 4.5 },
  { name: "Fit mat", role: "button" as const, min: 4.5 },
  { name: "Properties", role: "tab" as const, min: 4.5 },
  { name: "Undo", role: "button" as const, min: 3 },
];

async function openSwap(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await page.evaluate(() => (window as unknown as { __holdPass: () => void }).__holdPass());
  await startCut(page);
  await page.evaluate(() => (window as unknown as { __releasePass: () => void }).__releasePass());
  await expect(page.getByRole("region", { name: "Colour swap" })).toBeVisible();
}

function jobInkLocators(page: Page) {
  return JOB_INK.map((item) => ({
    ...item,
    locator: page.getByRole(item.role, { name: item.name, exact: true }),
  }));
}

test("a cold dark boot paints job controls on the full-page shot", async ({ page }) => {
  test.setTimeout(60_000);
  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(installMockTauri, { seedRedGold: true });
  await page.addInitScript(() => {
    localStorage.setItem("cuthulhu.theme", "dark");
    sessionStorage.setItem("cuthulhu.preferences", JSON.stringify({ appearance: "dark", units: "mm" }));
  });
  await openSwap(page);
  await expect(page.locator("html")).toHaveClass(/dark/);
  const samples = await contrastsOnPageShot(page, [
    ...jobInkLocators(page),
    { name: "Rulers", locator: page.getByRole("button", { name: "Rulers", exact: true }), inset: 4 },
  ]);
  assertDarkJobInk(samples.filter((sample) => sample.name !== "Rulers"));
  assertFilledInk(samples.filter((sample) => sample.name === "Rulers"));
});

test("a light-to-dark switch paints job controls on the screenshot the walk takes", async ({ page }) => {
  test.setTimeout(60_000);
  await page.emulateMedia({ colorScheme: "light" });
  await page.addInitScript(installMockTauri, { seedRedGold: true });
  await openSwap(page);
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  const items = [
    ...jobInkLocators(page),
    { name: "Rulers", min: 4.5, locator: page.getByRole("button", { name: "Rulers", exact: true }), inset: 4 },
  ];
  const hit = await flipDarkShot(page, items);
  assertDarkJobInk(hit.filter((sample) => sample.name !== "Rulers"));
  assertFilledInk(hit.filter((sample) => sample.name === "Rulers"));
});

test("pressed, secondary and send-sheet buttons clear 4.5:1 on the full-page shot", async ({ page }) => {
  test.setTimeout(90_000);
  await page.addInitScript(installMockTauri, { seedRedGold: true });

  await page.emulateMedia({ colorScheme: "light" });
  assertFilledInk(await filledInkShot(page));

  await page.emulateMedia({ colorScheme: "dark" });
  await page.addInitScript(() => {
    localStorage.setItem("cuthulhu.theme", "dark");
    sessionStorage.setItem("cuthulhu.preferences", JSON.stringify({ appearance: "dark", units: "mm" }));
  });
  assertFilledInk(await filledInkShot(page));

  await page.emulateMedia({ colorScheme: "light" });
  await page.addInitScript(() => {
    localStorage.setItem("cuthulhu.theme", "light");
    sessionStorage.setItem("cuthulhu.preferences", JSON.stringify({ appearance: "light", units: "mm" }));
  });
  const light = await filledSurfaces(page);
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  // Boxes before the click. Anything awaited after it lets a transition finish,
  // which is the frame the full-page shot does not get. The sheet covers the
  // top bar, so the menu is opened from the element rather than the pointer.
  const confirmHit = await flipDarkShot(page, [
    { name: "Cancel", locator: light.cancel, inset: 4 },
    { name: "Rulers", locator: light.rulers, inset: 4 },
  ]);
  assertFilledInk(confirmHit);
});

test("a pressed pill's fill differs from its container in both themes", async ({ page }) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  await assertPressedApart(page, "light");

  await page.addInitScript(() => {
    localStorage.setItem("cuthulhu.theme", "dark");
    sessionStorage.setItem("cuthulhu.preferences", JSON.stringify({ appearance: "dark", units: "mm" }));
  });
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await expect(page.locator("html")).toHaveClass(/dark/);
  await assertPressedApart(page, "dark");
});

test("the send sheet secondary clears 4.5:1 on the theme-flip frame", async ({ page }) => {
  test.setTimeout(60_000);
  await page.emulateMedia({ colorScheme: "light" });
  await page.addInitScript(installMockTauri, { seedRedGold: true });
  await page.addInitScript(() => {
    localStorage.setItem("cuthulhu.theme", "light");
    sessionStorage.setItem("cuthulhu.preferences", JSON.stringify({ appearance: "light", units: "mm" }));
  });
  const host = await openSendHost(page);
  await expect(page.locator("html")).not.toHaveClass(/dark/);
  assertFilledInk(await flipDarkShot(page, [{ name: "Add a Cut Host…", locator: host, inset: 4 }], "class"));
});

async function openSendHost(page: Page) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Send to cutter", exact: true }).click();
  const host = page.getByRole("dialog", { name: "Send to cutter" }).getByRole("button", { name: "Add a Cut Host…", exact: true });
  await expect(host).toBeVisible();
  return host;
}

async function filledSurfaces(page: Page) {
  const host = await openSendHost(page);
  const hostInk = await contrastsOnPageShot(page, [{ name: "Add a Cut Host…", locator: host, inset: 4 }]);
  await page.getByRole("radio", { name: "Silhouette Cameo 5 Alpha", exact: true }).first().click();
  await page.evaluate(() => (window as unknown as { __holdPass: () => void }).__holdPass());
  await startCut(page);
  await page.evaluate(() => (window as unknown as { __releasePass: () => void }).__releasePass());
  await callFakeWith(page, "__test_set_status", {
    phase: "AwaitingConfirmation",
    actions: { cancel: true, confirm: true },
    rest: { pass: { index: 1, total: 2 }, sent: { sent: 100, total: 100 } },
  });
  const cancel = page.getByRole("region", { name: "Job" }).getByRole("button", { name: "Cancel", exact: true });
  const rulers = page.getByRole("button", { name: "Rulers", exact: true });
  await expect(cancel).toBeVisible();
  const confirmInk = await contrastsOnPageShot(page, [
    { name: "Cancel", locator: cancel, inset: 4 },
    { name: "Rulers", locator: rulers, inset: 4 },
  ]);
  return { hostInk, confirmInk, cancel, rulers, all: [...hostInk, ...confirmInk] };
}

async function filledInkShot(page: Page) {
  const surfaces = await filledSurfaces(page);
  return surfaces.all;
}

async function flipDarkShot(
  page: Page,
  items: { name: string; locator: Locator; min?: number; inset?: number }[],
  how: "menu" | "class" = "menu",
) {
  const boxed: { name: string; min?: number; box: InkBox["box"] }[] = [];
  for (const item of items) {
    const box = await item.locator.boundingBox();
    if (!box) throw new Error(`no box for ${item.name}`);
    boxed.push({ name: item.name, min: item.min, box: insetBox(box, item.inset ?? 0) });
  }
  if (how === "class") {
    // The sheet is modal, so the top bar is hidden from the tree and the
    // Appearance click cannot land. The menu's paint is this class change.
    await page.evaluate(() => {
      document.documentElement.classList.add("dark");
      document.documentElement.style.colorScheme = "dark";
    });
  } else {
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("menuitemradio", { name: "Dark", exact: true }).click();
  }
  const bytes = (await page.screenshot()) as unknown as Uint8Array;
  const crops = await contrastsInPng(page, boxed, bytes);
  return crops.map((crop, i) => ({ name: crop.name, min: boxed[i].min, hit: crop.hit }));
}

function assertDarkJobInk(samples: { name: string; min?: number; hit: ReturnType<typeof renderedContrastFromPixels> }[]) {
  for (const sample of samples) {
    const min = sample.min ?? JOB_INK.find((item) => item.name === sample.name)?.min ?? 4.5;
    console.log("job ink", sample.name, sample.hit.ratio.toFixed(2), sample.hit.ink.join(","), "on", sample.hit.background.join(","));
    expect(sample.hit.background[0], `${sample.name} background`).toBeLessThan(80);
    expect(sample.hit.ratio, sample.name).toBeGreaterThanOrEqual(min);
  }
}

function assertFilledInk(samples: { name: string; hit: ReturnType<typeof renderedContrastFromPixels> }[]) {
  for (const sample of samples) {
    console.log("filled ink", sample.name, sample.hit.ratio.toFixed(2), sample.hit.ink.join(","), "on", sample.hit.background.join(","));
    expect(sample.hit.ratio, sample.name).toBeGreaterThanOrEqual(4.5);
  }
}

/** The padding band above the pill, inside the control that holds it. Skipping
 *  the parent's own 1px border keeps that edge out of the container sample. */
function containerBand(parent: InkBox["box"], pill: InkBox["box"]): InkBox["box"] {
  const y = parent.y + 1;
  return {
    x: parent.x + 2,
    y,
    width: Math.max(1, parent.width - 4),
    height: Math.max(1, pill.y - y),
  };
}

/** The top of the pill, wide enough that a rounded corner is not the sample.
 *  Two rows cover a 1px border when the box origin rounds onto the row above. */
function edgeBand(pill: InkBox["box"]): InkBox["box"] {
  return {
    x: pill.x + pill.width * 0.25,
    y: pill.y,
    width: Math.max(1, pill.width * 0.5),
    height: 2,
  };
}

function rowRgb(rgba: number[], width: number, y: number): Rgb {
  const start = y * width * 4;
  return modeRgb(rgba.slice(start, start + width * 4));
}

async function assertPressedApart(page: Page, theme: "light" | "dark") {
  const pills = [
    { name: "mm", locator: page.getByRole("button", { name: "mm", exact: true }) },
    { name: "Rulers", locator: page.getByRole("button", { name: "Rulers", exact: true }) },
    { name: "Layers", locator: page.getByRole("tab", { name: "Layers", exact: true }) },
  ];
  const regions: InkBox[] = [];
  for (const pill of pills) {
    const box = await pill.locator.boundingBox();
    const parent = await pill.locator.locator("xpath=..").boundingBox();
    if (!box || !parent) throw new Error(`no box for ${pill.name}`);
    regions.push({ name: `${pill.name} fill`, box: insetBox(box, 4) });
    regions.push({ name: `${pill.name} container`, box: containerBand(parent, box) });
    regions.push({ name: `${pill.name} edge`, box: edgeBand(box) });
  }
  const bytes = (await page.screenshot()) as unknown as Uint8Array;
  const crops = await readPngRegions(page, regions, bytes);
  for (let i = 0; i < pills.length; i++) {
    const name = pills[i].name;
    const fillCrop = crops[i * 3];
    const containerCrop = crops[i * 3 + 1];
    const edgeCrop = crops[i * 3 + 2];
    const text = renderedContrastFromPixels(fillCrop.rgba);
    const fill = modeRgb(fillCrop.rgba);
    const container = modeRgb(containerCrop.rgba);
    const edges = Array.from({ length: edgeCrop.height }, (_, y) => rowRgb(edgeCrop.rgba, edgeCrop.width, y));
    const sep = edges.map((edge) => pillApart(fill, container, edge)).find((hit) => hit.apart) ?? pillApart(fill, container, edges[0]);
    console.log(
      theme,
      name,
      "text",
      text.ratio.toFixed(2),
      text.ink.join(","),
      "fill",
      sep.fillRatio.toFixed(2),
      fill.join(","),
      "on",
      container.join(","),
      "edge",
      sep.edgeRatio.toFixed(2),
    );
    expect(text.ratio, `${theme} ${name} text`).toBeGreaterThanOrEqual(4.5);
    expect(sep.apart, `${theme} ${name} fill ${sep.fillRatio.toFixed(2)} edge ${sep.edgeRatio.toFixed(2)}`).toBe(true);
    if (theme === "dark") {
      expect(Math.min(...fill), `${theme} ${name} stays a white pill`).toBeGreaterThanOrEqual(250);
    } else {
      expect(Math.min(...fill), `${theme} ${name} is not a white pill`).toBeLessThan(250);
    }
  }
}

// Emits an event the *backend* sends, once the page is actually listening for it.
//
// The wait is on the fake's own registration rather than on an element being visible: `goto`
// resolves at load, `App` subscribes from a `useEffect`, and an emit that arrives before that has
// no listener to reach — which is a test that fails for its timing rather than for the sentence it
// is about. A rendered button is a proxy for the same commit having happened; the registration is
// the thing itself.
const emitBackendEvent = async (page: Page, event: string) => {
  await expect
    .poll(() =>
      page.evaluate((e) => {
        const internals = window as unknown as {
          __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
        };
        return internals.__TAURI_INTERNALS__.invoke("__test_backend_listeners", { event: e });
      }, event),
    )
    .toBeGreaterThan(0);
  await expect
    .poll(() => page.evaluate(() => sessionStorage.getItem("__close_guard_ready__")))
    .toBe("true");
  await page.evaluate((e) => {
    const internals = window as unknown as {
      __TAURI_INTERNALS__: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown> };
    };
    return internals.__TAURI_INTERNALS__.invoke("__test_emit_backend_event", { event: e });
  }, event);
};

// The close guard's own half of #158 is Rust's, and tested there. This is the sentence it produces:
// quitting stops what this process sends and leaves a Cut Host's Job cutting, so a prompt that
// implies the quit stops everything would be telling the operator the opposite of what happens —
// and `cancel` is cooperative, so even the local cutter finishes its buffered moves (the Silhouette
// has no abort command; see apps/desktop/MANUAL-CHECKLIST.md).
test("the quit prompt says what quitting stops and what it leaves running", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.evaluate(() => {
    sessionStorage.setItem("__dialog_answer__", "ok");
    sessionStorage.setItem("__dialog_hold__", "1");
  });

  await emitBackendEvent(page, "cut-in-progress");

  const prompts = async () =>
    JSON.parse((await page.evaluate(() => sessionStorage.getItem("__dialog_messages__"))) ?? "[]") as string[];
  await expect.poll(async () => (await prompts()).length).toBe(1);
  const [prompt] = await prompts();
  expect(prompt).toContain("Sending to the cutter on this computer stops");
  expect(prompt).toContain("already buffered");
  expect(prompt).toContain("Cut Host keeps running there");
  // The answer is held by the fake until released below, so this is an ordering and not a race
  // against a timer: nothing may have happened yet. The prompt this replaced took a Promise
  // for a yes and quit here.
  expect(await page.evaluate(() => sessionStorage.getItem("__force_quit__"))).toBeNull();
  expect(await page.evaluate(() => sessionStorage.getItem("__quit_cancelled__"))).toBeNull();
  await page.evaluate(() => sessionStorage.setItem("__dialog_released__", "1"));
  // Accepting it is what quits, and the fake records having been asked.
  await expect
    .poll(() => page.evaluate(() => sessionStorage.getItem("__force_quit__")))
    .toBe("asked");
  // Confirming is not a cancel: telling the backend both would reply to the OS twice.
  expect(await page.evaluate(() => sessionStorage.getItem("__quit_cancelled__"))).toBeNull();

  // The handshake's own ordering, which the prompt above cannot see: readiness is stood down
  // before anything is subscribed, and never announced while nothing is listening. Rust emits on
  // the strength of this flag and `emit` succeeds with zero listeners, so a `true` with no listener
  // is a prevented close the operator cannot answer — and the leading `false` is the only thing
  // that closes the window a reload leaves open, since no effect cleanup runs on unload.
  //
  // A filter over the log rather than an exact sequence: Playwright runs the dev build, whose
  // `StrictMode` mounts, cleans up and mounts again, so more than two entries is correct here.
  const readiness = (await page.evaluate(() => sessionStorage.getItem("__ready_log__")))!.split(",");
  expect(readiness[0]).toBe("false@0");
  expect(readiness).not.toContain("true@0");

  // The teardown direction is not observable here — Playwright cannot unmount React — and is
  // pinned instead by `closeGuard.test.ts`, over the same handshake this page runs.
});

test("dismissing the quit prompt leaves the window open and quits nothing", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.evaluate(() => {
    sessionStorage.setItem("__dialog_answer__", "cancel");
    sessionStorage.setItem("__dialog_hold__", "1");
  });

  await emitBackendEvent(page, "cut-in-progress");

  // Held unanswered: neither door has been told anything yet.
  await expect
    .poll(async () =>
      JSON.parse((await page.evaluate(() => sessionStorage.getItem("__dialog_messages__"))) ?? "[]") as string[],
    )
    .toHaveLength(1);
  expect(await page.evaluate(() => sessionStorage.getItem("__quit_cancelled__"))).toBeNull();
  expect(await page.evaluate(() => sessionStorage.getItem("__force_quit__"))).toBeNull();
  await page.evaluate(() => sessionStorage.setItem("__dialog_released__", "1"));

  // Dismissing is told to the backend, because on the Dock's Quit or a logout the OS is waiting on
  // the answer: a cancel that stayed in the page would leave a logout hanging on this app. It is
  // also the beat that proves the answer was read, so the absence checks below mean something.
  await expect
    .poll(() => page.evaluate(() => sessionStorage.getItem("__quit_cancelled__")))
    .toBe("asked");
  expect(await page.evaluate(() => sessionStorage.getItem("__force_quit__"))).toBeNull();
  await expect(page.getByRole("button", { name: "Send to cutter", exact: true })).toBeVisible();
});

function phase7Shots(): boolean {
  return (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env.PHASE7_SHOTS === "1";
}

async function phaseShot(page: Page, name: string) {
  if (!phase7Shots()) return;
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.screenshot({ path: `/opt/cursor/artifacts/phase7-${name}-light.png` });
  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Dark", exact: true }).click();
  await page.screenshot({ path: `/opt/cursor/artifacts/phase7-${name}-dark.png` });
  await page.getByRole("button", { name: "Appearance" }).click();
  await page.getByRole("menuitemradio", { name: "Light", exact: true }).click();
}

const VINYL_BOARD = ["Gold vinyl", "Red vinyl", "Black vinyl", "White vinyl", "Navy vinyl", "Purple vinyl", "#f5e6a3"] as const;

test("layers follow the colour passes and a cut toggle stays on the row", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedVinylBoard: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await showTab(page, "Layers");

  await expect(page.getByTestId("colour-layer")).toHaveCount(VINYL_BOARD.length);
  for (const name of VINYL_BOARD) {
    await expect(page.getByRole("button", { name: `Cut ${name}`, exact: true })).toHaveAttribute("aria-pressed", "true");
  }
  await expect(page.getByText("Colour layers", { exact: true })).toBeVisible();
  await expect(page.getByText("Each colour is cut as its own pass. Drag a shape to reorder it among its siblings.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Cut this shape", exact: true })).toHaveCount(0);

  const red = page.getByTestId("colour-layer").filter({ has: page.getByRole("button", { name: "Cut Red vinyl", exact: true }) });
  const gold = page.getByTestId("colour-layer").filter({ has: page.getByRole("button", { name: "Cut Gold vinyl", exact: true }) });
  await red.getByRole("button").nth(1).click();
  await expect(red.getByRole("button").nth(1)).toHaveAttribute("aria-current", "true");
  await expect(red.getByRole("button").nth(1)).not.toHaveAttribute("aria-pressed");
  await expect(page.getByTestId("layer-row").filter({ hasText: "Ellipse 2" })).toHaveAttribute("data-selected", "true");
  await expect(page.getByTestId("layer-row").filter({ hasText: "Ellipse 1" })).toHaveAttribute("data-selected", "false");
  await gold.getByRole("button").nth(1).click({ modifiers: ["Shift"] });
  await expect(page.getByTestId("layer-row").filter({ hasText: "Ellipse 1" })).toHaveAttribute("data-selected", "true");
  await expect(page.getByTestId("layer-row").filter({ hasText: "Ellipse 2" })).toHaveAttribute("data-selected", "true");

  for (const name of VINYL_BOARD) {
    if (name === "Red vinyl") continue;
    await page.getByRole("button", { name: `Collapse ${name}`, exact: true }).click();
  }
  await expect(page.getByTestId("layer-row").filter({ hasText: "Ellipse 2" })).toBeVisible();
  await expect(page.getByText("Ellipse 1", { exact: true })).toHaveCount(0);
  await phaseShot(page, "layers-one-open");

  await page.getByRole("button", { name: "Cut Red vinyl", exact: true }).click();
  await expect(page.getByRole("button", { name: "Cut Red vinyl", exact: true })).toHaveAttribute("aria-pressed", "false");
  await expect(page.getByTestId("layer-row").filter({ hasText: "Ellipse 2" })).toBeVisible();
  await phaseShot(page, "layers-cut-off");
});

test("properties show one shape in millimetres and in inches", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedMachine: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");

  await expect(page.getByLabel("X", { exact: true })).toHaveValue("0");
  await expect(page.getByLabel("Y", { exact: true })).toHaveValue("0");
  await expect(page.getByLabel("W", { exact: true })).toHaveValue("10");
  await expect(page.getByLabel("H", { exact: true })).toHaveValue("10");
  await expect(page.getByLabel("Rotation", { exact: true })).toHaveValue("0");
  await expect(page.getByLabel("Design for")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Document", exact: true })).toHaveCount(0);
  await expect(page.getByLabel("Cut this shape", { exact: true })).toBeChecked();
  await phaseShot(page, "properties-mm");

  await page.getByRole("group", { name: "Units" }).getByRole("button", { name: "in", exact: true }).click();
  await expect(page.getByLabel("W", { exact: true })).toHaveValue("0.394");
  await expect(page.getByLabel("X", { exact: true })).toHaveValue("0");
  await expect(page.getByLabel("Rotation", { exact: true })).toHaveValue("0");
  await phaseShot(page, "properties-in");
});

test("a mixed selection keeps align and does not pick a material", async ({ page }) => {
  await page.addInitScript(installMockTauri, {
    seedTwoColorRects: true,
    seedMachine: true,
    seedUserPreset: true,
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").first().click();
  await showTab(page, "Properties");
  await page.getByLabel("Material preset").selectOption("preset:card-stock");
  await showTab(page, "Layers");
  await page.getByTestId("layer-row").nth(1).click({ modifiers: ["Shift"] });
  await showTab(page, "Properties");

  await expect(page.getByLabel("Material preset").locator("option").first()).toHaveText("Mixed");
  await expect(page.getByLabel("Material preset")).toHaveValue("");
  await expect(page.getByRole("button", { name: "Align left edges", exact: true })).toBeVisible();
  await expect(page.getByLabel("X", { exact: true })).toHaveCount(0);
  await expect(page.getByLabel("Design for")).toBeVisible();
  await phaseShot(page, "properties-mixed");
});

test("an empty selection shows the document, not a second units control", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedTwoColorRects: true, seedMachine: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await showTab(page, "Properties");

  const section = page.getByRole("region", { name: "Document" });
  await expect(section.getByRole("heading", { name: "Document", exact: true })).toBeVisible();
  await expect(section.getByText("Mat", { exact: true }).locator("xpath=following-sibling::span")).toHaveText("12 × 12 in (305 × 305 mm)");
  await expect(section.getByText("Units", { exact: true }).locator("xpath=following-sibling::span")).toHaveText("mm");
  await expect(section.getByText("Select an object to edit its size and position.")).toBeVisible();
  await expect(section.getByText("Colour passes", { exact: true }).locator("xpath=following-sibling::span")).toHaveText("2");
  await expect(section.getByText("Objects", { exact: true }).locator("xpath=following-sibling::span")).toHaveText("2");
  await expect(section.getByLabel("Design for")).toBeVisible();
  await expect(section.getByRole("button", { name: "mm", exact: true })).toHaveCount(0);
  await expect(section.getByLabel("Mat size")).toHaveCount(0);
  await expect(page.getByLabel("Mat size")).toHaveCount(1);
  await expect(page.getByLabel("X", { exact: true })).toHaveCount(0);
  await phaseShot(page, "document");
});

test("a nearly invisible shape still says it cuts", async ({ page }) => {
  await page.addInitScript(installMockTauri, { seedFaint: true });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await showTab(page, "Layers");
  await expect(page.getByRole("button", { name: "Cut Red vinyl", exact: true })).toBeVisible();
  await page.getByTestId("layer-row").click();
  await showTab(page, "Properties");
  await expect(page.getByText("Nearly invisible in the file. It still cuts.")).toBeVisible();
  await expect(page.getByLabel("Cut this shape", { exact: true })).toBeChecked();
  await phaseShot(page, "faint");
});
