// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { isEditingTarget, nudgeFromKey } from "./nudge";

const plain = { shiftKey: false, altKey: false, metaKey: false, ctrlKey: false };

describe("nudgeFromKey", () => {
  it("moves 1 mm, 10 mm with Shift, and 0.1 mm with Alt", () => {
    expect(nudgeFromKey({ ...plain, key: "ArrowRight", code: "ArrowRight" })).toEqual({ t: "move", dx: 1, dy: 0 });
    expect(nudgeFromKey({ ...plain, key: "ArrowUp", code: "ArrowUp", shiftKey: true })).toEqual({ t: "move", dx: 0, dy: -10 });
    expect(nudgeFromKey({ ...plain, key: "ArrowLeft", code: "ArrowLeft", altKey: true })).toEqual({ t: "move", dx: -0.1, dy: 0 });
  });

  it("rotates 15° on a bracket and 1° with Shift, using the physical key", () => {
    // Shift+[ is "{" on a US layout, so the character is not the signal.
    expect(nudgeFromKey({ ...plain, key: "{", code: "BracketLeft", shiftKey: true })).toEqual({ t: "rotate", deg: -1 });
    expect(nudgeFromKey({ ...plain, key: "]", code: "BracketRight" })).toEqual({ t: "rotate", deg: 15 });
  });

  it("leaves ⌘ and Ctrl chords alone", () => {
    expect(nudgeFromKey({ ...plain, key: "ArrowLeft", code: "ArrowLeft", metaKey: true })).toBeNull();
    expect(nudgeFromKey({ ...plain, key: "z", code: "KeyZ", ctrlKey: true })).toBeNull();
  });
});

describe("isEditingTarget", () => {
  it("is false for a target that is not an element", () => {
    expect(isEditingTarget(null)).toBe(false);
    expect(isEditingTarget({ tagName: "INPUT" } as unknown as EventTarget)).toBe(false);
  });

  it("is true for a field and for anything inside a dialog", () => {
    const input = { tagName: "INPUT", isContentEditable: false, closest: () => null };
    expect(isEditingTarget(input as unknown as EventTarget)).toBe(true);
    const inside = { tagName: "DIV", isContentEditable: false, closest: (sel: string) => (sel.includes("dialog") ? {} : null) };
    expect(isEditingTarget(inside as unknown as EventTarget)).toBe(true);
    const canvas = { tagName: "CANVAS", isContentEditable: false, closest: () => null };
    expect(isEditingTarget(canvas as unknown as EventTarget)).toBe(false);
  });
});
