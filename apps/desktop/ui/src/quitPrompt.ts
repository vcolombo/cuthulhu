// SPDX-License-Identifier: GPL-3.0-or-later
import { confirmed } from "./dialogAnswer";

/**
 * What the operator is asked when the backend says a cut may still be running, and what each
 * answer does.
 *
 * The question is asked through the dialog plugin, which is asynchronous. `window.confirm` is not
 * an option on desktop: the plugin replaces it with an `async` function, so the "answer" is a
 * Promise, which is truthy whichever button is pressed. The prompt this replaces therefore quit on
 * every Cancel. Pulled out of the effect that holds it for the same reason `closeGuard.ts` is: the
 * order of "wait for the answer, then act" is the invariant, and an effect body cannot be asserted.
 */
export const QUIT_WARNING =
  "A cut may still be running — quit anyway? Sending to the cutter on this computer " +
  "stops, but it finishes whatever moves it has already buffered; a Job already sent " +
  "to a Cut Host keeps running there.";

export type QuitPromptDeps = {
  /** Ask the operator; resolves `true` only for "Quit anyway". */
  ask: (message: string) => Promise<boolean>;
  /** Quit now. */
  forceQuit: () => Promise<void>;
  /** Stay open, and release a macOS terminate the OS is waiting on. */
  cancelQuit: () => Promise<void>;
  /** Where a failure goes: a quit prompt that broke is not silent. */
  onError: (e: unknown) => void;
};

/**
 * Ask, wait for the answer, then act on it.
 *
 * Only a resolved `true` quits. A question that could not be asked or answered is a no, because
 * the alternative is quitting over a cut nobody agreed to abandon, and a "stay" is always
 * recoverable. It is still told to the backend: on the Dock's Quit or a logout the OS is waiting
 * for a reply, and a prompt that failed would otherwise leave it waiting.
 */
export async function answerQuitPrompt({ ask, forceQuit, cancelQuit, onError }: QuitPromptDeps): Promise<void> {
  const quit = await confirmed(() => ask(QUIT_WARNING), onError);
  if (!quit) {
    try {
      await cancelQuit();
    } catch (e) {
      onError(e);
    }
    return;
  }
  try {
    await forceQuit();
  } catch (e) {
    onError(e);
  }
}
