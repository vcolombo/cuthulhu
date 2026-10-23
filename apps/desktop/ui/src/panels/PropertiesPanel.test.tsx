// SPDX-License-Identifier: GPL-3.0-or-later
// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, test, vi } from "vitest";
import { PropertiesPanel, showDocumentSection } from "./PropertiesPanel";

const onChangeX = vi.fn();

function props(over: Partial<Parameters<typeof PropertiesPanel>[0]> = {}): Parameters<typeof PropertiesPanel>[0] {
  return {
    bounds: null,
    cutLineType: null,
    materialPreset: undefined,
    effectiveMaterial: { kind: "one", id: null },
    presets: [],
    onChangeX,
    onChangeY: () => {},
    onChangeW: () => {},
    onChangeH: () => {},
    rotation: null,
    onChangeRotation: () => {},
    unit: "mm",
    unitCount: 0,
    editsLocked: null,
    distributeBlocked: { x: "few", y: "few" },
    onAlign: () => {},
    onDistribute: () => {},
    onChangeCutLineType: () => {},
    onChangeMaterialPreset: () => {},
    machines: [{ id: "cameo5", name: "Silhouette Cameo 5", width_mm: 330, height_mm: 3000 }],
    machineId: "cameo5",
    onDesignFor: () => {},
    documentInfo: { matLabel: "12 × 12 in (305 × 305 mm)", passCount: 3, objectCount: 4 },
    ...over,
  };
}

describe("document section", () => {
  let host: HTMLDivElement;
  let root: Root;

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
    onChangeX.mockReset();
  });

  function render(over: Partial<Parameters<typeof PropertiesPanel>[0]> = {}) {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root.render(createElement(PropertiesPanel, props(over)));
    });
    return host;
  }

  test("nothing selected shows the document, not the shape fields", () => {
    expect(showDocumentSection(null, null, undefined)).toBe(true);
    const panel = render();
    expect(panel.querySelector("h2")?.textContent).toBe("Document");
    const fact = (label: string) => {
      const span = [...panel.querySelectorAll("span")].find((el) => el.textContent === label);
      return span?.nextElementSibling?.textContent;
    };
    expect(fact("Mat")).toBe("12 × 12 in (305 × 305 mm)");
    expect(fact("Units")).toBe("mm");
    expect(fact("Passes")).toBe("3");
    expect(fact("Objects")).toBe("4");
    expect(panel.querySelector("[aria-label='Design for']")).not.toBeNull();
    expect([...panel.querySelectorAll("label")].some((label) => label.textContent?.trim() === "X")).toBe(false);
  });

  test("a selection keeps Design for and shows lengths in the chosen unit", () => {
    const panel = render({
      bounds: { x: 25.4, y: 0, w: 50.8, h: 10 },
      cutLineType: "Cut",
      materialPreset: { state: "inherit" },
      unit: "in",
      rotation: 0,
    });
    expect(showDocumentSection({ x: 0, y: 0, w: 1, h: 1 }, "Cut", { state: "inherit" })).toBe(false);
    expect(panel.querySelector("h2")).toBeNull();
    const field = (name: string) => {
      const label = [...panel.querySelectorAll("label")].find((el) => el.querySelector("span")?.textContent === name);
      return label?.querySelector("input") as HTMLInputElement;
    };
    expect(field("X").value).toBe("1");
    expect(field("W").value).toBe("2");
    expect(field("Rotation").value).toBe("0");
    expect(onChangeX).not.toHaveBeenCalled();
  });

  test("a mixed material selection keeps the align row and the Mixed option", () => {
    const panel = render({
      bounds: null,
      cutLineType: "mixed",
      materialPreset: "mixed",
      unitCount: 2,
      distributeBlocked: { x: null, y: "few" },
      faintVinyl: true,
    });
    expect(panel.querySelector("h2")).toBeNull();
    expect(panel.querySelector("[aria-label='Align left edges']")).not.toBeNull();
    expect(panel.querySelector("[aria-label='Material preset']")?.querySelector("option")?.textContent).toBe("Mixed");
    expect(panel.textContent).toContain("Nearly invisible");
  });
});
