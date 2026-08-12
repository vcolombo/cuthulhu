// SPDX-License-Identifier: GPL-3.0-or-later
import { ipcErrorMessage, type DeviceInfo, type ExistingPairing, type PairedHostView } from "../ipc";

/** Where the flow is, and everything the dialog draws. Fields belong to the kinds that set
 *  them: `fingerprint` and `existing` from `confirm` onward, `devices`/`host` on `paired`,
 *  `message` on `failed`. Flat rather than a discriminated union because every consumer already
 *  switches on `kind`, and a union would charge a narrowing for each field read without adding a
 *  fact. */
export type PairingState = {
  kind: "idle" | "probing" | "confirm" | "testing" | "paired" | "failed";
  fingerprint?: string;
  /** A host already paired at this address, if there is one. */
  existing?: ExistingPairing;
  devices?: DeviceInfo[];
  host?: PairedHostView;
  message?: string;
};

/** `name` is what the operator will see in the device list; it defaults to the address. */
export type PairingInput = { address: string; token: string; name?: string };

/** Injected so the flow can be tested without a backend. The dialog passes
 *  `probeHost`/`testHost`/`pairHost` straight through. */
export type PairingEffects = {
  probe: (address: string) => Promise<string>;
  test: (address: string, token: string, fingerprint: string) => Promise<DeviceInfo[]>;
  save: (name: string, address: string, token: string, fingerprint: string) => Promise<PairedHostView>;
  /** Whether this address is already paired, and whether the probed certificate is the pinned
   *  one. Asked between the probe and the confirm, which is the last moment before the operator
   *  types a token they may not need to type at all. */
  existing: (address: string, fingerprint: string) => Promise<ExistingPairing | null>;
};

export type PairingOptions = {
  /** The operator's answer to the fingerprint. A function may park until they click. */
  confirmFingerprint: boolean | ((fingerprint: string) => boolean | Promise<boolean>);
  onState?: (state: PairingState) => void;
};

/**
 * Pair a Cut Host, trust-on-first-use: probe, confirm, test, save.
 *
 * The order is the security property, not a rendering detail. `probe` sends no token, so the
 * fingerprint reaches the operator before this side has vouched for anything; only once they
 * confirm it does `test` carry the token. Reject at the confirm step and nothing that knows the
 * token has spoken to whatever answered at that address.
 *
 * Nothing is saved until `test` reached the host and listed its cutters — a pairing that saves
 * first and discovers later is how one Pi ends up with two entries.
 *
 * A pairing against an address already paired says so at the confirm step, and says it loudly when
 * the certificate has changed as well. Pairing again cannot be refused — a changed fingerprint is
 * a hard refusal on every later connection, so re-pairing is the only recovery there is — but the
 * result is a second row, identical to the first but for which of them errors, and nothing
 * anywhere explaining why (#107). The warning is the whole of the fix, and the confirm step is
 * where it belongs: after the fingerprint is known, before a token is sent.
 *
 * Every message rendered from the returned state is the Rust side's own prose, unaltered (#94).
 */
export async function runPairing(
  input: PairingInput,
  effects: PairingEffects,
  options: PairingOptions,
): Promise<PairingState> {
  const publish = (state: PairingState): PairingState => {
    options.onState?.(state);
    return state;
  };

  publish({ kind: "probing" });
  let fingerprint: string;
  try {
    fingerprint = await effects.probe(input.address);
  } catch (e) {
    return publish({ kind: "failed", message: ipcErrorMessage(e) });
  }

  // Swallowed rather than raised: not knowing whether this address is already paired is a reason
  // to show one less warning, never a reason to block a pairing that would otherwise work.
  const existing = await effects.existing(input.address, fingerprint).catch(() => null);

  publish({ kind: "confirm", fingerprint, existing: existing ?? undefined });
  const answer = options.confirmFingerprint;
  const confirmed = typeof answer === "function" ? await answer(fingerprint) : answer;
  // Back to the form, with nothing sent and nothing stored.
  if (!confirmed) return publish({ kind: "idle" });

  publish({ kind: "testing", fingerprint, existing: existing ?? undefined });
  let devices: DeviceInfo[];
  try {
    devices = await effects.test(input.address, input.token, fingerprint);
  } catch (e) {
    return publish({ kind: "failed", fingerprint, existing: existing ?? undefined, message: ipcErrorMessage(e) });
  }

  try {
    const host = await effects.save(input.name ?? input.address, input.address, input.token, fingerprint);
    return publish({ kind: "paired", fingerprint, existing: existing ?? undefined, devices, host });
  } catch (e) {
    return publish({ kind: "failed", fingerprint, existing: existing ?? undefined, message: ipcErrorMessage(e) });
  }
}
