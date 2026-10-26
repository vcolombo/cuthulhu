// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { answerQuitPrompt, confirmed, QUIT_WARNING } from "./quitPrompt";

/**
 * The question is answered later than it is asked, which is the whole difference from the
 * `window.confirm` it replaced: the dialog plugin's version returns a Promise, truthy whichever
 * button is pressed, so the old code quit on Cancel. Every answer here is delayed behind a latch
 * the test opens, so "acted before the operator answered" is something that can be observed.
 */
const harness = () => {
  const log: string[] = [];
  let answer!: (yes: boolean) => void;
  let fail!: (e: unknown) => void;
  const asked = new Promise<boolean>((resolve, reject) => {
    answer = resolve;
    fail = reject;
  });
  const deps = {
    log,
    ask: (message: string) => {
      log.push(`ask:${message === QUIT_WARNING ? "warning" : message}`);
      return asked;
    },
    forceQuit: async () => void log.push("force"),
    cancelQuit: async () => void log.push("cancel"),
    onError: (e: unknown) => void log.push(`error:${e}`),
  };
  return { deps, answer: (yes: boolean) => answer(yes), fail: (e: unknown) => fail(e) };
};

/** Drains the chain of awaits inside `answerQuitPrompt`. */
const settle = async () => {
  for (let i = 0; i < 8; i++) await Promise.resolve();
};

describe("answerQuitPrompt", () => {
  it("does nothing until the operator has answered", async () => {
    const h = harness();
    const done = answerQuitPrompt(h.deps);
    await settle();

    // A truthy Promise treated as "yes" would already have quit here.
    expect(h.deps.log).toEqual(["ask:warning"]);
    h.answer(false);
    await done;
  });

  it("quits on a later yes, and does not cancel", async () => {
    const h = harness();
    const done = answerQuitPrompt(h.deps);
    await settle();
    h.answer(true);
    await done;

    expect(h.deps.log).toEqual(["ask:warning", "force"]);
  });

  it("stays open on a later no, and tells the backend so", async () => {
    const h = harness();
    const done = answerQuitPrompt(h.deps);
    await settle();
    h.answer(false);
    await done;

    // `cancel` is what releases a deferred macOS terminate; without it a logout waits on this app.
    expect(h.deps.log).toEqual(["ask:warning", "cancel"]);
  });

  it("treats a question that could not be asked as a no, not a yes", async () => {
    const h = harness();
    const done = answerQuitPrompt(h.deps);
    await settle();
    h.fail("dialog unavailable");
    await done;

    expect(h.deps.log).toEqual(["ask:warning", "error:dialog unavailable", "cancel"]);
  });

  it("only a literal true quits", async () => {
    const h = harness();
    const done = answerQuitPrompt({ ...h.deps, ask: async () => ({}) as unknown as boolean });
    await done;

    expect(h.deps.log).toEqual(["cancel"]);
  });

  it("reports a failed quit once, and does not also cancel", async () => {
    const h = harness();
    const done = answerQuitPrompt({
      ...h.deps,
      forceQuit: async () => {
        throw "exit refused";
      },
    });
    await settle();
    h.answer(true);
    await done;

    expect(h.deps.log).toEqual(["ask:warning", "error:exit refused"]);
  });

  it("reports a failed cancel without rejecting", async () => {
    const h = harness();
    const done = answerQuitPrompt({
      ...h.deps,
      cancelQuit: async () => {
        h.deps.log.push("cancel-attempted");
        throw "no backend";
      },
    });
    await settle();
    h.answer(false);
    // Awaiting it is the assertion: App fires this and forgets it, so a rejection is unhandled.
    await done;

    expect(h.deps.log).toEqual(["ask:warning", "cancel-attempted", "error:no backend"]);
  });

  it("reports both failures when the question and the cancel both fail", async () => {
    const h = harness();
    const done = answerQuitPrompt({
      ...h.deps,
      cancelQuit: async () => {
        throw "no backend";
      },
    });
    await settle();
    h.fail("dialog unavailable");
    await done;

    expect(h.deps.log).toEqual(["ask:warning", "error:dialog unavailable", "error:no backend"]);
  });
});
describe("confirmed", () => {
  it("waits for the answer, and a delayed no is a no", async () => {
    const h = harness();
    const pending = confirmed(() => h.deps.ask("any"), h.deps.onError);
    await settle();
    h.answer(false);
    await expect(pending).resolves.toBe(false);
  });

  it("a delayed yes is a yes", async () => {
    const h = harness();
    const pending = confirmed(() => h.deps.ask("any"), h.deps.onError);
    await settle();
    h.answer(true);
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
