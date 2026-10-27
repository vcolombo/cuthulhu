// SPDX-License-Identifier: GPL-3.0-or-later

/** What an operator reads when a toast is the surface for a stale plan. The sheet's own
 *  banner says the same thing and does not also toast; this is the wording anywhere else. */
export const STALE_PLAN_COPY = "Your design changed after the cut was planned. Replan to continue.";

/** The backend's stale-plan sentences, in the words an operator reads. Anything else is
 *  already the message to show. */
export function toastCopy(message: string): string {
  if (/changed since (?:this |the )?cut was planned/i.test(message) || /unrecognized plan revision/.test(message)) {
    return STALE_PLAN_COPY;
  }
  return message;
}
