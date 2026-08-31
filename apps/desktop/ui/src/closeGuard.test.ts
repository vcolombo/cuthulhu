// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it, vi } from "vitest";
import { armCloseGuard } from "./closeGuard";

/**
 * One recorder for both sides of the handshake, because the defect is always an order rather than
 * a value: `ready:true` before `listen`, or `unlisten` before `ready:false`.
 */
const recorder = () => {
  const log: string[] = [];
  let unlistened = false;
  const deps = {
    log,
    setReady: async (ready: boolean) => {
      log.push(`ready:${ready}`);
    },
    listen: async (_onWarning: () => void) => {
      log.push("listen");
      return () => {
        unlistened = true;
        log.push("unlisten");
      };
    },
    onWarning: () => {},
    onError: (e: unknown) => log.push(`error:${e}`),
    get unlistened() {
      return unlistened;
    },
  };
  return deps;
};

/** The chain is three promises deep in each direction; this drains them. */
const settle = async () => {
  for (let i = 0; i < 8; i++) await Promise.resolve();
};

describe("armCloseGuard", () => {
  it("stands readiness down before subscribing, and announces it only after", async () => {
    const r = recorder();
    armCloseGuard(r);
    await settle();

    // The leading `false` is what closes the reload window: native readiness survives the page
    // that set it, and no effect cleanup runs on unload.
    expect(r.log).toEqual(["ready:false", "listen", "ready:true"]);
  });

  it("clears readiness before removing the listener", async () => {
    const r = recorder();
    const teardown = armCloseGuard(r);
    await settle();

    teardown();
    await settle();

    // Reversed, the backend holds `true` with nobody listening: its emit succeeds, it prevents the
    // close, and the operator has a window that will not shut and no prompt to answer.
    expect(r.log).toEqual(["ready:false", "listen", "ready:true", "ready:false", "unlisten"]);
  });

  it("never announces readiness for a listener it has already given up", async () => {
    const r = recorder();
    const teardown = armCloseGuard(r);
    // Torn down while `setReady(false)`/`listen` are still in flight — React's StrictMode does
    // exactly this on every mount in development.
    teardown();
    await settle();

    expect(r.log).not.toContain("ready:true");
    expect(r.unlistened).toBe(true);
  });

  it("reports a handshake that could not be armed", async () => {
    const log: string[] = [];
    armCloseGuard({
      setReady: async () => {
        throw new Error("backend gone");
      },
      listen: async () => () => {},
      onWarning: () => {},
      onError: (e) => log.push(`error:${(e as Error).message}`),
    });
    await settle();

    // A guard that cannot be armed is the operator's business: without this the window silently
    // loses its only warning.
    expect(log).toEqual(["error:backend gone"]);
  });

  it("passes the warning through to the prompt", async () => {
    const onWarning = vi.fn();
    let handler: (() => void) | null = null;
    armCloseGuard({
      setReady: async () => {},
      listen: async (h) => {
        handler = h;
        return () => {};
      },
      onWarning,
      onError: () => {},
    });
    await settle();

    handler!();
    expect(onWarning).toHaveBeenCalledOnce();
  });
});
