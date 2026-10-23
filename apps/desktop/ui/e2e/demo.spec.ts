// SPDX-License-Identifier: GPL-3.0-or-later
import { expect, test, type Page } from "@playwright/test";
import ipcInventory from "../../ipc-inventory.json" with { type: "json" };
import { DEMO_FIXTURES, DEMO_STORAGE_KEY, IMPORT_FAILED, type DemoId } from "../src/demo/fixtures";
import { installMockTauri } from "./mockBackend";

// Copied from smoke.spec.ts. Importing that file would register its tests in this project.
test.beforeEach(async ({ page }) => {
  await page.addInitScript((inventory) => {
    const injected = window as unknown as { __IPC_INVENTORY__: unknown };
    injected.__IPC_INVENTORY__ = Object.assign(Object.create(null), inventory);
  }, ipcInventory as Record<string, string[]>);
});

test.afterEach(async ({ page }) => {
  if (!page.url().startsWith("http")) return;
  const refused = await page.evaluate(() => {
    const stored: unknown = JSON.parse(sessionStorage.getItem("__ipc_violations__") ?? "[]");
    return Array.isArray(stored) ? stored : [];
  });
  expect(refused, "the fake refused an IPC call the desktop's registered commands do not declare").toEqual([]);
});

async function boot(page: Page, id: DemoId) {
  await page.addInitScript(installMockTauri, { demo: { id } });
  await page.goto("/");
}

async function expectState(page: Page, id: DemoId) {
  switch (id) {
    case "normal":
      await expect(page.getByTestId("layer-row")).toHaveCount(2);
      await expect(page.getByRole("heading", { name: "Your mat is empty" })).toHaveCount(0);
      await expect(page.getByRole("button", { name: "No cutter · Add one" })).toHaveCount(0);
      break;
    case "noCutters":
      await showDevices(page);
      await expect(page.getByText("No cutters yet.")).toBeVisible();
      await expect(page.getByRole("button", { name: "No cutter · Add one" })).toBeVisible();
      break;
    case "emptyCanvas":
      await expect(page.getByRole("heading", { name: "Your mat is empty" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Import SVG" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Trace an image" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Draw a rectangle" })).toBeVisible();
      await expect(page.getByTestId("layer-row")).toHaveCount(0);
      break;
    case "importFailed": {
      const toast = page.locator("[data-sonner-toast]");
      await expect(toast).toBeVisible();
      await expect(toast).toContainText(IMPORT_FAILED);
      // The enter fade is 400ms. A shot or a contrast read before it settles sees the layers through the card.
      await expect(toast).toHaveCSS("opacity", "1");
      break;
    }
    case "hostOffline":
      await expect(page.getByText(/Connection lost · last update/)).toBeVisible();
      await expect(page.getByText(/Reconnecting to Workshop Pi/)).toBeVisible();
      break;
    default: {
      const never: never = id;
      throw new Error(`unhandled demo state: ${never}`);
    }
  }
}

async function showDevices(page: Page) {
  const tab = page.getByRole("tab", { name: "Devices", exact: true });
  if ((await tab.getAttribute("data-state")) !== "active") await tab.click();
}

async function setAppearance(page: Page, mode: "Light" | "Dark") {
  const trigger = page.getByRole("button", { name: "Appearance", exact: true });
  await expect(trigger).toBeEnabled();
  await trigger.click();
  await page.getByRole("menuitemradio", { name: mode, exact: true }).click();
  const root = page.locator("html");
  if (mode === "Dark") await expect(root).toHaveClass(/dark/);
  else await expect(root).not.toHaveClass(/dark/);
}

test("normal shows the seeded shapes and the connected-cutter controls", async ({ page }) => {
  await boot(page, "normal");
  await expectState(page, "normal");
});

test("no cutters shows the empty devices list and the add-cutter pill", async ({ page }) => {
  await boot(page, "noCutters");
  await expectState(page, "noCutters");
});

test("empty canvas shows the mat card and no shapes", async ({ page }) => {
  await boot(page, "emptyCanvas");
  await expectState(page, "emptyCanvas");
});

test("import failed shows the toast from the shared fixture", async ({ page }) => {
  await boot(page, "importFailed");
  await expectState(page, "importFailed");
});

test("host offline shows the job banner for the unreachable host", async ({ page }) => {
  await boot(page, "hostOffline");
  await expectState(page, "hostOffline");
});

test("the dev menu reloads into the fixture it writes", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Demo state", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Demo state", exact: true }).click();
  await page.getByRole("menuitem", { name: "Empty canvas", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Your mat is empty" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Demo state: Empty canvas" })).toBeVisible();
  const stored = await page.evaluate((key) => sessionStorage.getItem(key), DEMO_STORAGE_KEY);
  expect(JSON.parse(stored ?? "null")).toEqual({ id: "emptyCanvas" });
});

test("demo states in light and dark", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  const shot = async (name: string) => {
    await page.screenshot({ path: testInfo.outputPath(`${name}.png`) });
  };

  await page.addInitScript(installMockTauri, { demo: { id: "normal" } });
  await page.goto("/");
  await expect(page.getByRole("button", { name: /Demo state/ })).toBeVisible();
  for (const mode of ["Light", "Dark"] as const) {
    await setAppearance(page, mode);
    await page.getByRole("button", { name: /Demo state/ }).click();
    await expect(page.getByRole("menuitem", { name: "Normal", exact: true })).toBeVisible();
    await shot(`demo-menu-${mode.toLowerCase()}`);
    await page.keyboard.press("Escape");
  }

  for (const fixture of DEMO_FIXTURES) {
    // Only the demo key. sessionStorage.clear() would drop __ipc_violations__ before afterEach reads it.
    await page.evaluate((key) => sessionStorage.removeItem(key), DEMO_STORAGE_KEY);
    await page.addInitScript(installMockTauri, { demo: { id: fixture.id } });
    await page.goto("/");
    await expectState(page, fixture.id);
    for (const mode of ["Light", "Dark"] as const) {
      await setAppearance(page, mode);
      await expectState(page, fixture.id);
      await shot(`demo-${fixture.id}-${mode.toLowerCase()}`);
    }
  }
});
