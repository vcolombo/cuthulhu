// SPDX-License-Identifier: GPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { STALE_PLAN_COPY, toastCopy } from "./toastCopy";

describe("toastCopy", () => {
  it("turns a stale-plan refusal into a sentence", () => {
    expect(toastCopy("document changed since the cut was planned; replan")).toBe(STALE_PLAN_COPY);
    expect(toastCopy("the document changed since this cut was planned")).toBe(STALE_PLAN_COPY);
    expect(toastCopy("cut request carries an unrecognized plan revision")).toBe(STALE_PLAN_COPY);
    expect(toastCopy("travel request carries an unrecognized plan revision")).toBe(STALE_PLAN_COPY);
  });

  it("leaves every other refusal as the backend wrote it", () => {
    expect(toastCopy("no fonts are installed")).toBe("no fonts are installed");
  });
});
