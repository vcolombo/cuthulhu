// SPDX-License-Identifier: GPL-3.0-or-later
/**
 * What the dialog plugin's answer to a confirmation means, for every caller that asks one.
 *
 * `window.confirm` is not an option on desktop: the plugin replaces it with an `async` function,
 * so its "answer" is a Promise, truthy whichever button is pressed. Its replacement is asked
 * through `ipc` and judged here, so the quit prompt and the machine switch share one policy and
 * neither has to import the other's module to get it.
 *
 * Whether a confirmation the operator could answer should be asked at all, and what an answer
 * that never arrived means. It takes an asynchronous `ask` and returns `false` for a rejected
 * question, so the one shape a caller wants to guard against ("the dialog did not show, so
 * proceed") is unrepresentable at the call site.
 */
export async function confirmed(ask: () => Promise<boolean>, onError: (e: unknown) => void): Promise<boolean> {
  try {
    return (await ask()) === true;
  } catch (e) {
    onError(e);
    return false;
  }
}
