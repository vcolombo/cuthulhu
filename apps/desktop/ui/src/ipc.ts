// SPDX-License-Identifier: GPL-3.0-or-later
import { invoke } from "@tauri-apps/api/core";

// ponytail: loose types for delta/snapshot payloads until Rust shape mirrors TS
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Args = Record<string, any>;

export async function newDoc() {
  return invoke("newDoc", {});
}

export async function snapshot() {
  return invoke("snapshot", {});
}

export async function commitTransform(args: Args) {
  return invoke("commitTransform", args);
}

export async function addPrimitive(args: Args) {
  return invoke("addPrimitive", args);
}

export async function booleanOp(args: Args) {
  return invoke("booleanOp", args);
}

export async function addText(args: Args) {
  return invoke("addText", args);
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
  return invoke("importSvg", args);
}

export async function saveProject(args: Args) {
  return invoke("saveProject", args);
}

export async function loadProject(args: Args) {
  return invoke("loadProject", args);
}

export async function setMachine(args: Args) {
  return invoke("setMachine", args);
}

export async function listMachines() {
  return invoke("listMachines", {});
}
