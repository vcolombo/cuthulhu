// SPDX-License-Identifier: GPL-3.0-or-later
import { test, expect } from "@playwright/test";

// Minimal in-memory fake Tauri backend. Runs inside the page (via addInitScript, so it
// can't close over anything outside itself) and mirrors the JSON shape produced by
// crates/document's Document::snapshot_json() — see App.tsx's DocSnapshot/buildScene,
// which is what actually parses this on the JS side.
function installMockTauri() {
  type Node = { id: number; kind: unknown; transform: number[]; style: unknown; children: number[] };
  type Doc = {
    nodes: Record<number, Node>;
    root: number;
    artboard: { x: number; y: number; w: number; h: number };
    machine: { id: string; name: string; width_mm: number; height_mm: number } | null;
  };

  const machines = [
    { id: "cameo5_alpha", name: "Silhouette Cameo 5 Alpha", width_mm: 330, height_mm: 3000 },
    { id: "puma_iv", name: "GCC Puma IV", width_mm: 600, height_mm: 5000 },
  ];

  let nextId = 1;
  const freshDoc = (): Doc => {
    const rootId = nextId++;
    return {
      nodes: { [rootId]: { id: rootId, kind: "Layer", transform: [1, 0, 0, 1, 0, 0], style: {}, children: [] } },
      root: rootId,
      artboard: { x: 0, y: 0, w: 330, h: 3000 },
      machine: null,
    };
  };
  let doc = freshDoc();
  let saved: Doc | null = null;

  const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
    new_doc: () => JSON.stringify((doc = freshDoc())),
    snapshot: () => JSON.stringify(doc),
    add_primitive: (a) => {
      const id = nextId++;
      doc.nodes[id] = { id, kind: { Shape: a.kind }, transform: [1, 0, 0, 1, 0, 0], style: {}, children: [] };
      doc.nodes[a.parent as number].children.push(id);
      return {};
    },
    add_text: (a) => {
      const id = nextId++;
      doc.nodes[id] = { id, kind: { Shape: { Path: { d: "" } } }, transform: [1, 0, 0, 1, 0, 0], style: {}, children: [] };
      doc.nodes[a.parent as number].children.push(id);
      return {};
    },
    commit_transform: (a) => {
      const m = a.m as number[];
      for (const id of a.ids as number[]) {
        const t = doc.nodes[id]?.transform;
        if (t) {
          t[4] += m[4];
          t[5] += m[5];
        }
      }
      return {};
    },
    delete: (a) => {
      for (const id of a.ids as number[]) {
        delete doc.nodes[id];
        for (const n of Object.values(doc.nodes)) n.children = n.children.filter((c) => c !== id);
      }
      return {};
    },
    reorder: () => ({}),
    undo: () => null,
    redo: () => null,
    boolean_op: () => ({}),
    import_svg: () => [{}, []],
    save_project: () => {
      saved = JSON.parse(JSON.stringify(doc));
      return null;
    },
    load_project: () => {
      if (saved) doc = JSON.parse(JSON.stringify(saved));
      return JSON.stringify(doc);
    },
    set_machine: (a) => {
      const m = machines.find((p) => p.id === a.machineId);
      if (!m) throw new Error("unknown machine");
      doc.machine = m;
      doc.artboard = { x: 0, y: 0, w: m.width_mm, h: m.height_mm };
      return null;
    },
    list_machines: () => machines,
  };

  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
    invoke: (cmd: string, args: Record<string, unknown> = {}) => {
      const fn = commands[cmd];
      if (!fn) return Promise.reject(new Error(`unmocked command: ${cmd}`));
      try {
        return Promise.resolve(fn(args));
      } catch (e) {
        return Promise.reject(e instanceof Error ? e.message : String(e));
      }
    },
    transformCallback: () => 0,
  };
}

test("new doc → add rect → save → reload keeps the rect", async ({ page }) => {
  await page.addInitScript(installMockTauri);
  await page.goto("/");
  await page.getByRole("button", { name: "Rectangle" }).click();
  await page.mouse.click(400, 300);
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
  await page.getByRole("button", { name: "Save" }).click();
  await page.getByRole("button", { name: "Reload" }).click();
  await expect(page.getByTestId("layer-row")).toHaveCount(1);
});
