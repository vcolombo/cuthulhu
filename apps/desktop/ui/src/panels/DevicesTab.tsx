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
      <p className="m-0 p-3 text-xs text-destructive">{listError}</p>
    );
  }

  if (devices.length === 0) {
    return (
      <div className="flex flex-col gap-2 p-3 text-xs text-muted-foreground">
        <p style={{ margin: 0 }}>No cutters yet.</p>
        <p style={{ margin: 0 }}>To pair a Cut Host, open “Send to cutter”. A cutter plugged into this computer shows up here once it is listed.</p>
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
              <span className="block text-xs text-foreground">{nameOf(info.machine_id)}</span>
              <span className="block text-2xs text-muted-foreground">{whereOf(info)}</span>
            </span>
            <button
              type="button"
              disabled={aimed}
              onClick={() => onPick(info)}
              style={{
                background: "var(--panel)",
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
