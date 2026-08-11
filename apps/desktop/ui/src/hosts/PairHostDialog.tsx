// SPDX-License-Identifier: GPL-3.0-or-later
import { useRef, useState, type CSSProperties } from "react";
import * as ipc from "../ipc";
import { runPairing, type PairingState } from "./pairing";

type Props = {
  onPaired: (host: ipc.PairedHostView) => void;
  onClose: () => void;
};

const panelStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0,0,0,0.5)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 100,
};

const dialogStyle: CSSProperties = {
  background: "var(--panel)",
  border: "1px solid var(--border)",
  color: "var(--text)",
  padding: 16,
  width: 460,
  display: "flex",
  flexDirection: "column",
  gap: 10,
};

const btn: CSSProperties = {
  background: "var(--panel)",
  color: "var(--text)",
  border: "1px solid var(--border)",
  padding: "4px 10px",
  cursor: "pointer",
};

const fieldStyle: CSSProperties = { display: "flex", flexDirection: "column", gap: 4 };

export function PairHostDialog({ onPaired, onClose }: Props) {
  const [address, setAddress] = useState("");
  const [token, setToken] = useState("");
  const [name, setName] = useState("");
  const [state, setState] = useState<PairingState>({ kind: "idle" });
  // The operator answers the fingerprint with a click, so the confirm step parks on a promise
  // that the buttons below resolve.
  const decide = useRef<((ok: boolean) => void) | null>(null);

  const running = state.kind === "probing" || state.kind === "confirm" || state.kind === "testing";

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
    <div style={panelStyle} onClick={onClose}>
      <div style={dialogStyle} onClick={e => e.stopPropagation()}>
        <h2 style={{ margin: 0, fontSize: 16 }}>Pair a Cut Host</h2>

        <label style={fieldStyle}>
          Address
          <input value={address} placeholder="pi.local:7878" disabled={running}
                 onChange={e => setAddress(e.target.value)} />
        </label>
        <label style={fieldStyle}>
          Token
          <input value={token} type="password" disabled={running}
                 onChange={e => setToken(e.target.value)} />
        </label>
        <label style={fieldStyle}>
          Name (optional)
          <input value={name} placeholder={address || "Workshop Pi"} disabled={running}
                 onChange={e => setName(e.target.value)} />
        </label>

        {state.kind === "probing" && <p>Contacting the host…</p>}

        {state.kind === "confirm" && (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {/* Said before the fingerprint, not after: a changed certificate is the reason the
                existing pairing stopped working, and it is what the operator is here to fix. The
                second row this will create is unavoidable — pairing cannot overwrite another
                host's pinned fingerprint — so what is owed is an explanation, not a refusal. */}
            {state.existing && !state.existing.sameFingerprint && (
              <p role="alert" style={{ margin: 0, color: "var(--cut)" }}>
                “{state.existing.name}” is already paired at this address, and it presented a
                different certificate then. Either the Cut Host was reinstalled, or something else
                is answering at this address. Pairing again adds a second entry; the old one will
                never connect again, and is yours to forget.
              </p>
            )}
            {state.existing && state.existing.sameFingerprint && (
              <p style={{ margin: 0, color: "var(--muted)" }}>
                “{state.existing.name}” is already paired at this address, with this same
                certificate. Pairing again adds a second entry rather than replacing it.
              </p>
            )}
            {/* The token has not been sent yet, and will not be unless this is confirmed. */}
            <p style={{ margin: 0 }}>The host presented this fingerprint. Check that it matches the
              one shown on the Cut Host's own console before continuing.</p>
            <code style={{ wordBreak: "break-all" }}>{state.fingerprint}</code>
            <div style={{ display: "flex", gap: 8 }}>
              <button style={btn} onClick={() => answer(true)}>It matches — continue</button>
              <button style={btn} onClick={() => answer(false)}>Cancel</button>
            </div>
          </div>
        )}

        {state.kind === "testing" && <p>Testing the token…</p>}

        {state.kind === "paired" && (
          <div>
            <p style={{ margin: 0 }}>Paired with {state.host?.name}.</p>
            {state.devices?.length
              ? <ul>{state.devices.map(d => <li key={d.instance_id}>{d.machine_id} ({d.instance_id})</li>)}</ul>
              : <p>No cutters are attached to it yet.</p>}
          </div>
        )}

        {/* The Rust side owns this prose; rewording a refusal here would give it a second copy (#94). */}
        {state.kind === "failed" && <p role="alert">{state.message}</p>}

        <div style={{ display: "flex", gap: 8, justifyContent: "flex-end" }}>
          {state.kind === "paired"
            ? <button style={btn} onClick={onClose}>Done</button>
            : <>
                <button style={btn} onClick={onClose}>Cancel</button>
                <button style={btn} disabled={running || !address.trim() || !token} onClick={start}>Pair</button>
              </>}
        </div>
      </div>
    </div>
  );
}
