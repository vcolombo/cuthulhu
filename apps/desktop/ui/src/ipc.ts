// SPDX-License-Identifier: GPL-3.0-or-later
import { invoke } from "@tauri-apps/api/core";

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
