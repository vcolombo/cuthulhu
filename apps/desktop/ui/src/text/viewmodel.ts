// SPDX-License-Identifier: GPL-3.0-or-later

// The picker's whole state as a union rather than nullable fields: "no fonts installed"
// and "the listing call failed" are different situations an operator can act on, and a
// blank <select> would hide both.
export type FontListState =
  | { kind: "loading" }
  | { kind: "ready"; families: string[]; selected: string }
  | { kind: "empty" }
  | { kind: "error"; message: string };

export function fontsLoaded(families: string[]): FontListState {
  if (families.length === 0) return { kind: "empty" };
  return { kind: "ready", families, selected: families[0] };
}

// No-op unless the family is one the backend actually listed — don't invent state the
// backend never offered (same refusal philosophy as trace's controlsFromSpecs).
export function selectFamily(state: FontListState, family: string): FontListState {
  if (state.kind !== "ready" || !state.families.includes(family)) return state;
  return { ...state, selected: family };
}

export function canInsert(state: FontListState): boolean {
  return state.kind === "ready";
}
