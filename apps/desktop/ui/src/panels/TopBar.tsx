// SPDX-License-Identifier: GPL-3.0-or-later
import { useRef } from "react";
import {
  Check,
  ChevronDown,
  FileImage,
  FileUp,
  FolderOpen,
  Redo2,
  RotateCcw,
  Save,
  Scissors,
  Shapes,
  Undo2,
} from "lucide-react";
import type { MachineProfile } from "../App";
import { Button } from "../components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "../components/ui/dropdown-menu";
import { Tip } from "../components/Tip";
import type { LengthUnit } from "../length";
import { cn } from "../lib/cn";
import type { DeviceInfo, PairedHostView, Phase } from "../ipc";
import type { Appearance } from "../theme";
import { AppearanceMenu } from "./AppearanceMenu";

type Props = {
  canUndo: boolean;
  canRedo: boolean;
  onUndo: () => void;
  onRedo: () => void;
  onImportFile: (file: File) => void;
  onTrace: () => void;
  onSave: () => void;
  onOpen: () => void;
  onReload: () => void;
  canReload: boolean;
  unit: LengthUnit;
  onUnit: (unit: LengthUnit) => void;
  appearance: Appearance;
  appearanceReady: boolean;
  onAppearance: (next: Appearance) => void;
  device: DeviceInfo | null;
  profileId: string | null;
  machines: MachineProfile[];
  devices: DeviceInfo[];
  hosts: PairedHostView[];
  phase: Phase;
  onPick: (info: DeviceInfo) => void;
  onRefreshDevices: () => void;
  onSend: () => void;
};

function machineName(machines: MachineProfile[], id: string | null): string | null {
  if (id === null) return null;
  return machines.find((machine) => machine.id === id)?.name ?? id;
}

function whereOf(info: DeviceInfo, hosts: PairedHostView[]): string {
  if (info.host === null) return "USB · this computer";
  return hosts.find((host) => host.id === info.host)?.name ?? info.host;
}

function phaseDot(phase: Phase): string {
  if (phase === "Idle") return "bg-success";
  if (phase === "Failed") return "bg-destructive";
  if (phase === "Disconnected") return "bg-muted-foreground";
  return "bg-accent-ink";
}

export function TopBar(props: Props) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const designName = machineName(props.machines, props.profileId);
  const connectedName = props.device ? machineName(props.machines, props.device.machine_id) : null;
  const noCutters = props.devices.length === 0 && props.hosts.length === 0;

  return (
    <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-panel px-3">
      <div className="flex items-center gap-2 pr-1">
        <div className="grid size-7 place-items-center rounded-md bg-subtle">
          <Scissors className="size-4 text-foreground" aria-hidden />
        </div>
        <span className="text-lg font-semibold tracking-tight text-accent-ink">Cuthulhu</span>
      </div>
      <div className="h-5 w-px bg-border" />
      <Tip label="Undo" kbd="⌘Z" disabled={!props.canUndo}>
        <Button variant="ghost" size="sm" onClick={props.onUndo} disabled={!props.canUndo} aria-label="Undo">
          <Undo2 />
          Undo
        </Button>
      </Tip>
      <Tip label="Redo" kbd="⇧⌘Z" disabled={!props.canRedo}>
        <Button variant="ghost" size="sm" onClick={props.onRedo} disabled={!props.canRedo} aria-label="Redo">
          <Redo2 />
          Redo
        </Button>
      </Tip>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" aria-label="Import">
            <FileUp />
            Import
            <ChevronDown className="opacity-60" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-60">
          <DropdownMenuLabel className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
            Vector — cuts as-is
          </DropdownMenuLabel>
          <DropdownMenuItem
            onSelect={() => {
              fileInputRef.current?.click();
            }}
          >
            <Shapes />
            SVG
            <span className="ml-auto text-2xs text-muted-foreground">.svg</span>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuLabel className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
            Image — opens Trace
          </DropdownMenuLabel>
          <DropdownMenuItem onSelect={props.onTrace}>
            <FileImage />
            Image
            <span className="ml-auto text-2xs text-muted-foreground">Opens Trace</span>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <input
        ref={fileInputRef}
        type="file"
        accept=".svg,image/svg+xml"
        style={{ display: "none" }}
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) props.onImportFile(file);
        }}
      />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" aria-label="File">
            File
            <ChevronDown className="opacity-60" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-52">
          <DropdownMenuItem onSelect={props.onSave}>
            <Save />
            Save
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={props.onOpen}>
            <FolderOpen />
            Open
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={props.onReload} disabled={!props.canReload}>
            <RotateCcw />
            Revert to saved
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <div className="flex-1" />

      <div role="group" aria-label="Units" className="flex rounded-md border border-border bg-background p-0.5">
        {(["in", "mm"] as const).map((unit) => (
          <button
            key={unit}
            type="button"
            aria-pressed={props.unit === unit}
            onClick={() => props.onUnit(unit)}
            className={cn(
              "h-6 cursor-pointer rounded-sm px-2.5 text-xs",
              props.unit === unit ? "bg-primary-foreground text-primary" : "text-muted-foreground hover:text-foreground",
            )}
          >
            {unit}
          </button>
        ))}
      </div>

      <AppearanceMenu appearance={props.appearance} ready={props.appearanceReady} onAppearance={props.onAppearance} />

      {props.device && connectedName ? (
        <DropdownMenu
          onOpenChange={(open) => {
            if (open) props.onRefreshDevices();
          }}
        >
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex h-8 max-w-[240px] items-center gap-2 rounded-md border border-border bg-background pl-2.5 pr-2 text-left hover:bg-subtle"
              aria-label={`Active cutter: ${connectedName}`}
            >
              <span className={cn("size-2 shrink-0 rounded-full", phaseDot(props.phase))} aria-hidden />
              <span className="flex min-w-0 flex-col leading-tight">
                <span className="truncate text-xs font-medium text-foreground">{connectedName}</span>
                <span className="truncate text-2xs text-muted-foreground">{whereOf(props.device, props.hosts)}</span>
              </span>
              <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-64">
            <DropdownMenuLabel className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
              Cutters
            </DropdownMenuLabel>
            {props.devices.map((info) => {
              const aimed = info.instance_id === props.device?.instance_id && info.host === props.device?.host;
              const name = machineName(props.machines, info.machine_id) ?? info.machine_id;
              return (
                <DropdownMenuItem key={`${info.host ?? "local"}:${info.instance_id}`} onSelect={() => props.onPick(info)}>
                  <span className="min-w-0 flex-1 truncate">{name}</span>
                  <span className="text-2xs text-muted-foreground">{whereOf(info, props.hosts)}</span>
                  {aimed ? <Check className="size-3.5" /> : null}
                </DropdownMenuItem>
              );
            })}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : noCutters ? (
        <Button variant="secondary" size="sm" className="h-8" onClick={props.onSend}>
          No cutter · Add one
        </Button>
      ) : (
        <DropdownMenu
          onOpenChange={(open) => {
            if (open) props.onRefreshDevices();
          }}
        >
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex h-8 max-w-[280px] items-center gap-2 rounded-md border border-border bg-background px-2.5 text-left text-xs hover:bg-subtle"
            >
              <span className="truncate text-muted-foreground">
                {designName ? `Designing for: ${designName} · no cutter connected` : "No cutter connected"}
              </span>
              <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-64">
            <DropdownMenuLabel className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
              Cutters
            </DropdownMenuLabel>
            {props.devices.length === 0 ? (
              <DropdownMenuItem onSelect={props.onSend}>No cutter · Add one</DropdownMenuItem>
            ) : (
              props.devices.map((info) => {
                const name = machineName(props.machines, info.machine_id) ?? info.machine_id;
                return (
                  <DropdownMenuItem key={`${info.host ?? "local"}:${info.instance_id}`} onSelect={() => props.onPick(info)}>
                    <span className="min-w-0 flex-1 truncate">{name}</span>
                    <span className="text-2xs text-muted-foreground">{whereOf(info, props.hosts)}</span>
                  </DropdownMenuItem>
                );
              })
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      )}

      <Button className="h-8 px-4 font-semibold" onClick={props.onSend}>
        <Scissors />
        Send to cutter
      </Button>
    </header>
  );
}
