// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { confirmed } from "./dialogAnswer";

/**
 * A question answered later than it is asked, which is the plugin's shape and the whole point.
 * Executor form: the UI's `lib` is older than `Promise.withResolvers`, which `tsc` rejects here
 * even though vitest, which does not type-check, would run it.
 */
const latch = () => {
  let answer!: (yes: boolean) => void;
  const asked = new Promise<boolean>((resolve) => {
    answer = resolve;
  });
  return { ask: () => asked, answer };
};

/** Drains the chain of awaits inside `confirmed`. */
const settle = async () => {
  for (let i = 0; i < 8; i++) await Promise.resolve();
};

describe("confirmed", () => {
  it("waits for the answer, and a delayed no is a no", async () => {
    const q = latch();
    const pending = confirmed(q.ask, () => {});
    let settled = false;
    void pending.then(() => (settled = true));
    await settle();
    expect(settled, "decided before the operator answered").toBe(false);
    q.answer(false);
    await expect(pending).resolves.toBe(false);
  });

  it("a delayed yes is a yes", async () => {
    const q = latch();
    const pending = confirmed(q.ask, () => {});
    await settle();
    q.answer(true);
    await expect(pending).resolves.toBe(true);
  });

  it("a question that could not be asked is a no, reported once", async () => {
    const errors: string[] = [];
    const pending = confirmed(async () => {
      throw "dialog unavailable";
    }, (e) => errors.push(String(e)));
    await expect(pending).resolves.toBe(false);
    expect(errors).toEqual(["dialog unavailable"]);
  });

  it("only a literal true confirms", async () => {
    const pending = confirmed(async () => ({}) as unknown as boolean, () => {});
    await expect(pending).resolves.toBe(false);
  });
});
