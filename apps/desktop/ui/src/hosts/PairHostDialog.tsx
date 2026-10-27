// SPDX-License-Identifier: GPL-3.0-or-later
import { useRef, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { Button } from "../components/ui/button";
import { cn } from "../lib/cn";
import * as ipc from "../ipc";
import { runPairing, type PairingState } from "./pairing";

type Props = {
  onPaired: (host: ipc.PairedHostView) => void;
  onClose: () => void;
};

const STEPS = ["Address", "Token", "Fingerprint"] as const;

/**
 * The three steps are a reading of the flow, not a wizard that hides the next field.
 * The token is on screen before Pair, and it is still not sent until the fingerprint
 * is confirmed — that order lives in `runPairing`, and this dialog only shows it.
 */
export function PairHostDialog({ onPaired, onClose }: Props) {
  const [address, setAddress] = useState("");
  const [token, setToken] = useState("");
  const [name, setName] = useState("");
  const [state, setState] = useState<PairingState>({ kind: "idle" });
  // The operator answers the fingerprint with a click, so the confirm step parks on a promise
  // that the buttons below resolve.
  const decide = useRef<((ok: boolean) => void) | null>(null);

  const running = state.kind === "probing" || state.kind === "confirm" || state.kind === "testing";
  const step = state.kind === "confirm" || state.kind === "testing" || state.kind === "paired" ? 3 : 1;

  async function start() {
    const done = await runPairing(
      { address: address.trim(), token, name: name.trim() || undefined },
      { probe: ipc.probeHost, test: ipc.testHost, save: ipc.pairHost, existing: ipc.existingPairing },
      {
        confirmFingerprint: () => new Promise<boolean>(resolve => { decide.current = resolve; }),
        onState: setState,
      },
    );
    if (done.kind === "paired" && done.host) onPaired(done.host);
  }

  function answer(ok: boolean) {
    const resolve = decide.current;
    decide.current = null;
    resolve?.(ok);
  }

  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[100] bg-black/50" />
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[100] flex w-[min(460px,92vw)] -translate-x-1/2 -translate-y-1/2 flex-col gap-3 rounded-lg border border-border bg-panel p-4 text-foreground outline-none">
          <Dialog.Title className="text-base font-medium">Pair a Cut Host</Dialog.Title>
          <Dialog.Description className="text-sm text-muted-foreground">
            The token is sent only after the fingerprint matches the one on the host.
          </Dialog.Description>

          <ol className="m-0 flex list-none gap-2 p-0" aria-label="Pairing steps">
            {STEPS.map((title, index) => {
              const number = index + 1;
              const current = number === step && state.kind !== "paired";
              const done = number < step || state.kind === "paired";
              return (
                <li key={title} className="flex flex-1 items-center gap-1.5 text-xs">
                  <span className={cn(
                    "grid size-5 place-items-center rounded-full border",
                    current && "border-accent-ink text-accent-ink",
                    done && "border-success text-success",
                    !current && !done && "border-border text-muted-foreground",
                  )}>
                    {number}
                  </span>
                  <span className={current ? "text-foreground" : "text-muted-foreground"}>{title}</span>
                </li>
              );
            })}
          </ol>

          <label className="flex flex-col gap-1 text-sm text-foreground">
            Address
            <input
              value={address}
              placeholder="pi.local:7878"
              disabled={running}
              onChange={e => setAddress(e.target.value)}
              className="h-9 rounded-md border border-input bg-card px-2 text-sm text-foreground"
            />
          </label>
          <label className="flex flex-col gap-1 text-sm text-foreground">
            Token
            <input
              value={token}
              type="password"
              disabled={running}
              onChange={e => setToken(e.target.value)}
              className="h-9 rounded-md border border-input bg-card px-2 text-sm text-foreground"
            />
          </label>
          <label className="flex flex-col gap-1 text-sm text-foreground">
            Name (optional)
            <input
              value={name}
              placeholder={address || "Workshop Pi"}
              disabled={running}
              onChange={e => setName(e.target.value)}
              className="h-9 rounded-md border border-input bg-card px-2 text-sm text-foreground"
            />
          </label>

          {state.kind === "probing" && <p className="m-0 text-sm text-muted-foreground">Contacting the host…</p>}

          {state.kind === "confirm" && (
            <div className="flex flex-col gap-2">
              {/* Said before the fingerprint, not after: a changed certificate is the reason the
                  existing pairing stopped working, and it is what the operator is here to fix. The
                  second row this will create is unavoidable — pairing cannot overwrite another
                  host's pinned fingerprint — so what is owed is an explanation, not a refusal. */}
              {state.existing && !state.existing.sameFingerprint && (
                <p role="alert" className="m-0 text-xs text-destructive">
                  “{state.existing.name}” is already paired at this address, and the certificate
                  pinned for it is not the one just presented. Either the Cut Host was reinstalled,
                  or something else is answering at this address. Pairing again adds a second entry;
                  the old one will never connect again, and is yours to forget.
                </p>
              )}
              {state.existing && state.existing.sameFingerprint && (
                <p className="m-0 text-xs text-muted-foreground">
                  “{state.existing.name}” is already paired at this address, with this same
                  certificate. Pairing again adds a second entry rather than replacing it.
                </p>
              )}
              {/* The token has not been sent yet, and will not be unless this is confirmed. */}
              <p className="m-0 text-sm">The host presented this fingerprint. Check that it matches the
                one shown on the Cut Host's own console before continuing.</p>
              <code className="break-all text-sm text-foreground">{state.fingerprint}</code>
              <div className="flex gap-2">
                <Button variant="default" size="sm" onClick={() => answer(true)}>It matches — continue</Button>
                <Button variant="secondary" size="sm" onClick={() => answer(false)}>Cancel</Button>
              </div>
            </div>
          )}

          {state.kind === "testing" && <p className="m-0 text-sm text-muted-foreground">Testing the token…</p>}

          {state.kind === "paired" && (
            <div className="text-sm">
              <p className="m-0">Paired with {state.host?.name}.</p>
              {state.devices?.length
                ? <ul>{state.devices.map(d => <li key={d.instance_id}>{d.machine_id} ({d.instance_id})</li>)}</ul>
                : <p className="m-0">No cutters are attached to it yet.</p>}
            </div>
          )}

          {/* The Rust side owns this prose; rewording a refusal here would give it a second copy (#94). */}
          {state.kind === "failed" && <p role="alert" className="m-0 text-sm text-destructive">{state.message}</p>}

          <div className="flex justify-end gap-2">
            {state.kind === "paired"
              ? <Button variant="secondary" size="sm" onClick={onClose}>Done</Button>
              : <>
                  <Button variant="ghost" size="sm" onClick={onClose}>Cancel</Button>
                  <Button variant="default" size="sm" disabled={running || !address.trim() || !token} onClick={() => { void start(); }}>Pair</Button>
                </>}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
