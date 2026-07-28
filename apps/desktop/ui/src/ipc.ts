// SPDX-License-Identifier: GPL-3.0-or-later
import { invoke } from "@tauri-apps/api/core";
import { save as dialogSave, open as dialogOpen } from "@tauri-apps/plugin-dialog";

// ponytail: loose types for delta/snapshot payloads until Rust shape mirrors TS
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Args = Record<string, any>;

export async function newDoc() {
  return invoke("new_doc", {});
}

export async function snapshot() {
  return invoke("snapshot", {});
}

export async function commitTransform(args: Args) {
  return invoke("commit_transform", args);
}

export async function addPrimitive(args: Args) {
  return invoke("add_primitive", args);
}

export async function booleanOp(args: Args) {
  return invoke("boolean_op", args);
}

export async function addText(args: Args) {
  return invoke("add_text", args);
}

export async function deleteNodes(args: Args) {
  return invoke("delete", args);
}

export async function reorder(args: Args) {
  return invoke("reorder", args);
}

export async function undo() {
  return invoke("undo", {});
}

export async function redo() {
  return invoke("redo", {});
}

export async function importSvg(args: Args) {
  return invoke("import_svg", args);
}

export async function saveProject(args: Args) {
  return invoke("save_project", args);
}

export async function loadProject(args: Args) {
  return invoke("load_project", args);
}

export async function setMachine(args: Args) {
  return invoke("set_machine", args);
}

export async function listMachines() {
  return invoke("list_machines", {});
}

// --- device / cut / preset wire types (mirror driver-core::manager + desktop::device) ---

export type TransportKind =
  | { Usb: { locator: string } }
  | { Serial: { path: string; baud: number } };

export type DeviceInfo = {
  instance_id: string;
  machine_id: string;
  transport: TransportKind;
  candidate: boolean;
};

export type DeviceError =
  | "Disconnected"
  | "Busy"
  | "Timeout"
  | "WriteZero"
  | { Io: string };

export type Phase =
  | "Disconnected"
  | "Connecting"
  | "Disconnecting"
  | "Idle"
  | "Sending"
  | "AwaitingConfirmation"
  | "AwaitingColorSwap"
  | "Cancelling"
  | "Failed";

/** Mirrors driver_core::CutStatus. The phase says what is happening now; `ended`
 *  says how the last job finished, which no phase can — a finished cut and a
 *  cancelled one both rest on "Idle". Actions say which buttons are legal.
 *  Nothing here needs interpreting, and nothing needs remembering. */
export type CutStatus = {
  phase: Phase;
  ended: "Completed" | "Cancelled" | null;
  actions: { cut: boolean; cancel: boolean; resume: boolean; confirm: boolean };
  pass: { index: number; total: number } | null;
  sent: { sent: number; total: number } | null;
  error: DeviceError | null;
};

/** Mirrors CutStatus::disconnected() — what to show before the first status arrives. */
export const DISCONNECTED_STATUS: CutStatus = {
  phase: "Disconnected",
  ended: null,
  actions: { cut: false, cancel: false, resume: false, confirm: false },
  pass: null,
  sent: null,
  error: null,
};

// `StateChanged` carries no payload: the event's own `status` is what changed.
export type DeviceEventKind =
  | "StateChanged"
  | { Progress: { pass_index: number; submitted_bytes: number; total_bytes: number } }
  | { PassComplete: number }
  | "JobComplete"
  | { Failed: DeviceError };

export type DeviceEvent = { job_id: number; kind: DeviceEventKind; status: CutStatus };

export type PlanCutPassSummary = { color: number | null; shape_count: number; node_ids: number[] };

export type PlanCutResponse = {
  passes: PlanCutPassSummary[];
  skipped_no_stroke: number;
  doc_revision: string;
  travel: [number, number, number, number][];
};

export type IpcError = { code: string; message: string };

// Real IpcError-derived commands reject with the serialized {code,message}
// object itself (not a string) — String(e) on those yields "[object Object]".
// Older doc-editing commands still reject with a plain string. Handle both.
export function ipcErrorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

export function ipcErrorCode(e: unknown): string | null {
  if (e && typeof e === "object" && "code" in e) return String((e as { code: unknown }).code);
  return null;
}

export async function listDevices(): Promise<DeviceInfo[]> {
  return invoke("list_devices", {});
}

export async function connectDevice(info: DeviceInfo): Promise<void> {
  return invoke("connect_device", { info });
}

export async function disconnectDevice(): Promise<void> {
  return invoke("disconnect_device", {});
}

export async function getDeviceState(): Promise<CutStatus> {
  return invoke("get_device_state", {});
}

export async function getConnectedDevice(): Promise<DeviceInfo | null> {
  return invoke("get_connected_device", {});
}

export async function forceQuit(): Promise<void> {
  return invoke("force_quit", {});
}

export async function planCut(): Promise<PlanCutResponse> {
  return invoke("plan_cut", {});
}

export async function cut(request: Args): Promise<number> {
  return invoke("cut", { request });
}

export async function cancelCut(): Promise<void> {
  return invoke("cancel_cut", {});
}

export async function resumeCut(): Promise<void> {
  return invoke("resume_cut", {});
}

export async function confirmPassDone(): Promise<void> {
  return invoke("confirm_pass_done", {});
}

export async function listPresets(machineId: string) {
  return invoke("list_presets", { machineId });
}

export async function machineCaps(machineId: string) {
  return invoke("machine_caps", { machineId });
}

export async function savePreset(p: Args) {
  return invoke("save_preset", { p });
}

export async function deletePreset(id: string) {
  return invoke("delete_preset", { id });
}

const CUT_FILTER = [{ name: "cuthulhu project", extensions: ["cut"] }];

export async function pickSavePath(): Promise<string | null> {
  return dialogSave({ defaultPath: "cuthulhu-project.cut", filters: CUT_FILTER });
}

export async function pickOpenPath(): Promise<string | null> {
  const r = await dialogOpen({ multiple: false, filters: CUT_FILTER });
  return typeof r === "string" ? r : null;
}

// --- trace wire types ---

export type TraceControlsDto = {
  mode: "binary" | "color";
  speckle: number;
  smoothing: number;
  detail: number;
  colors: number;
};
// Mirrors trace::ControlSpec. No range, default, or step is written on this side — the whole point
// of the command below is that these numbers have one home.
export type ControlSpec = {
  name: "speckle" | "smoothing" | "detail" | "colors";
  label: string;
  help: string;
  min: number;
  max: number;
  step: number;
  default: number;
  colorOnly: boolean;
};
export type TraceControlSpecsDto = {
  controls: ControlSpec[];
  defaultMode: "binary" | "color";
  maxDim: number;
};
export type TraceResultDto = { svg: string; pathCount: number; widthPx: number; heightPx: number; downscaled: boolean };

export async function traceControls(): Promise<TraceControlSpecsDto> {
  return invoke("trace_controls", {});
}
export async function traceImage(args: { path: string; controls: TraceControlsDto }): Promise<TraceResultDto> {
  return invoke("trace_image", args);
}
export async function loadImagePreview(args: { path: string }): Promise<string> {
  return invoke("load_image_preview", args);
}
// Goes through Rust rather than the dialog plugin directly: the backend records what the user
// picked and refuses to trace anything else, so choosing the file here is what grants access.
export async function pickImagePath(): Promise<string | null> {
  return invoke("pick_image", {});
}
