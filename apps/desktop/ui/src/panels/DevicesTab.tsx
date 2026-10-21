// SPDX-License-Identifier: GPL-3.0-or-later
import type { MachineProfile } from "../App";
import type { DeviceInfo, PairedHostView } from "../ipc";

type Props = {
  devices: DeviceInfo[];
  hosts: PairedHostView[];
  active: DeviceInfo | null;
  machines: MachineProfile[];
  listError: string | null;
  onPick: (info: DeviceInfo) => void;
};

/**
 * Enough of a Devices tab for the pill and "Set as active" to share one store.
 * Pairing, host errors and the live list stay in Send until phase 6.
 */
export function DevicesTab({ devices, hosts, active, machines, listError, onPick }: Props) {
  const nameOf = (id: string) => machines.find((machine) => machine.id === id)?.name ?? id;
  const whereOf = (info: DeviceInfo) =>
    info.host === null ? "USB · this computer" : (hosts.find((host) => host.id === info.host)?.name ?? info.host);

  if (listError !== null && devices.length === 0) {
    return (
      <p style={{ margin: 0, padding: 12, fontSize: 12, color: "var(--cut)" }}>{listError}</p>
    );
  }

  if (devices.length === 0) {
    return (
      <div style={{ padding: 12, fontSize: 12, color: "var(--muted-foreground)", display: "flex", flexDirection: "column", gap: 8 }}>
        <p style={{ margin: 0 }}>No cutters yet.</p>
        <p style={{ margin: 0 }}>Pair a Cut Host from Send to cutter. A cutter plugged into this computer shows up here once it is listed.</p>
      </div>
    );
  }

  return (
    <ul style={{ listStyle: "none", margin: 0, padding: 8, display: "flex", flexDirection: "column", gap: 6 }}>
      {devices.map((info) => {
        const aimed = active !== null && active.instance_id === info.instance_id && active.host === info.host;
        return (
          <li
            key={`${info.host ?? "local"}:${info.instance_id}`}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "6px 8px",
              border: "1px solid var(--border)",
              borderRadius: 6,
              background: "var(--card)",
            }}
          >
            <span style={{ flex: 1, minWidth: 0 }}>
              <span style={{ display: "block", fontSize: 12, color: "var(--text)" }}>{nameOf(info.machine_id)}</span>
              <span style={{ display: "block", fontSize: 11, color: "var(--muted-foreground)" }}>{whereOf(info)}</span>
            </span>
            <button
              type="button"
              disabled={aimed}
              onClick={() => onPick(info)}
              style={{
                background: "var(--panel)",
                color: "var(--text)",
                border: "1px solid var(--border)",
                padding: "4px 8px",
                cursor: aimed ? "default" : "pointer",
                fontSize: 12,
              }}
            >
              {aimed ? "Active" : "Set as active"}
            </button>
          </li>
        );
      })}
    </ul>
  );
}
