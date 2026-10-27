// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
import * as ipc from "../../ipc";
import {
  copyDraft,
  draftFault,
  draftOf,
  editorMode,
  isDirty,
  newDraft,
  selectAfterDelete,
  toPreset,
  type PresetDraft,
} from "../presetDraft";
import { rowPresetLookup, type AimedPresets, type Caps, type Preset } from "../viewmodel";

// What the fields allow before any machine has been asked. Not a machine's claim —
// a placeholder that keeps passes editable offline. Preflight ignores speed/force a
// machine does not support, so an optimistic default here cannot mis-send anything.
const ALL_ENABLED: Caps = { supportsSpeed: true, supportsForce: true, needsOperatorPassConfirm: false };

function cutterKey(info: ipc.DeviceInfo | null): string {
  if (!info) return "";
  return `${info.host ?? ""}:${info.instance_id}`;
}

/** The aimed cutter's preset list, with the generation guard that keeps one aim's reply from
 *  installing under the next (PR #264, #274). `aiming` lives here because a connect is what
 *  moves the aim. */
export function usePresetsForAim({
  connected,
  ranges,
  onError,
  onConnect,
  onDisconnect,
  replan,
  clearPending,
}: {
  connected: ipc.DeviceInfo | null;
  ranges: ipc.SettingsRanges | null;
  onError: (msg: string) => void;
  /** Resolves true when the document was retargeted to this cutter, which stales an open plan. */
  onConnect: (info: ipc.DeviceInfo) => Promise<boolean>;
  onDisconnect: () => Promise<unknown>;
  replan: () => void;
  /** Drops a decision that was waiting on the draft. An aim change replaces the draft, so the
   *  question about the previous one is no longer the operator's. */
  clearPending: () => void;
}) {
  // The machine id rides along with the caps: `connected` can change before an
  // in-flight fetch resolves, and showing one machine's capability against another
  // is the exact defect this fetch was added to remove.
  const [capsFor, setCapsFor] = useState<{ machineId: string; caps: Caps } | null>(null);
  // The machine id rides along with the presets for the same reason it does with the caps above: a
  // list read for one cutter must never be shown against another. A write leaves a `list_presets`
  // out, and an operator who aims elsewhere while it is in flight would otherwise get the previous
  // cutter's entries — and a save would then write that draft under the new machine's id (Greptile
  // on PR #264).
  const [presetsFor, setPresetsFor] = useState<(AimedPresets & { machineId: string }) | null>(null);
  /** The preset being edited, and the stored entry it started as. Two values because the
   *  difference is what "unsaved changes" means, and every guard below asks for it. */
  const [draft, setDraft] = useState<PresetDraft | null>(null);
  const [baseline, setBaseline] = useState<PresetDraft | null>(null);
  /** A preset write the backend refused, in its own words, beside the draft it refused. */
  const [presetError, setPresetError] = useState<string | null>(null);
  /** Why the aimed cutter's own list could not be read, if it could not. Kept here rather than only
   *  raised, because it is what the section says in place of the editor. */
  const [presetListError, setPresetListError] = useState<string | null>(null);
  /** A connect or disconnect is in flight, so which cutter this section is editing is about to
   *  change. The controls are withheld for that window: `aimPresetsAt` drops the draft when the aim
   *  lands, and an edit typed in the meantime would go with it (Codex on PR #264). */
  const [aiming, setAiming] = useState(false);
  const [presetBusy, setPresetBusy] = useState(false);

  const caps = connected && capsFor?.machineId === connected.machine_id ? capsFor.caps : ALL_ENABLED;

  /** Which aim the section is on: the cutter, and how many times the aim has moved. A ref rather
   *  than state because a reply or a captured action has to recognise its own aim from inside its
   *  callback, where `connected` is the value it closed over.
   *
   *  The count is the identity, not the machine id. **A machine id is not an aim**: two aims at the
   *  same cutter share it, so a `list_presets` owed to a previous connection, and a continuation
   *  captured under it, both passed a check on the id alone — a reconnect showed the cached list as
   *  though it were freshly read, and a continuation restored the previous aim's entry as this
   *  aim's draft, which the next save wrote under this machine's id. Every guard below is a
   *  generation check now (round 6 on PR #264). */
  const presetAim = useRef({ generation: 0, machineId: null as string | null });
  const isCurrentAim = (generation: number) => presetAim.current.generation === generation;
  /** This aim's own entries, or null while no read has answered for it. Nothing of another aim's is
   *  ever shown: the list is cleared when the aim moves and installed only under the aim that asked
   *  for it.
   *
   *  The two facts below are read off this one value because an empty list alone cannot tell a
   *  cutter with no presets from a cutter nobody has answered for, and every reader needs a
   *  different one of them (#267). */
  const aimedPresets: AimedPresets | null =
    connected !== null && presetsFor?.machineId === connected.machine_id ? presetsFor : null;
  /** What the pass rows resolve a material in. Not the same question the editor below asks: a row
   *  may only price a pass from a list that is still the file's answer, and the editor needs the
   *  entries a new one has to avoid whether or not a write has superseded them (#274). */
  const presetLookup = rowPresetLookup(aimedPresets);
  /** The entries this aim holds, superseded or not — what the editor writes against. */
  const presets = aimedPresets?.presets ?? [];
  /** Whether the editor may be shown at all: `freshPresetId` and the name check both read the list
   *  to avoid colliding with what is already stored, so a New pressed against a list that has not
   *  arrived can mint an id that already exists and overwrite that entry (Codex on PR #264). A
   *  superseded list still answers that, so a save does not take the editor off screen. */
  const presetsLoaded = aimedPresets !== null;

  const presetMode = editorMode(draft, presets);
  const presetDirty = draft !== null && baseline !== null && isDirty(draft, baseline);
  /** A builtin is never faulted because it is never written: its fields are read-only, and Save as
   *  Copy is what turns one into something editable. */
  const presetFault =
    draft === null || ranges === null || presetMode === "builtin"
      ? null
      : draftFault(draft, presets, ranges);

  /** Aims the section at a cutter, or at none, and answers with the aim it opened so a caller can
   *  hand it to the read that follows. A preset belongs to one machine, so the draft goes with the
   *  aim — and so does the list, because "loaded" has to mean *this* aim's list and not the one a
   *  previous connection to the same cutter left behind. */
  const aimPresetsAt = (machineId: string | null) => {
    const generation = presetAim.current.generation + 1;
    presetAim.current = { generation, machineId };
    setPresetsFor(null);
    setDraft(null);
    setBaseline(null);
    setPresetError(null);
    setPresetListError(null);
    clearPending();
    return generation;
  };

  /** The list, installed only under the aim that asked for it. A list that arrives is also what
   *  answers the previous read's failure: held past it, the section would say the presets cannot be
   *  read while holding the ones it just read, and only a disconnect would clear it (Copilot on
   *  PR #264). */
  const installPresets = (generation: number, machineId: string, list: unknown) => {
    if (!isCurrentAim(generation)) return false;
    setPresetsFor({ machineId, presets: list as Preset[], current: true });
    setPresetListError(null);
    return true;
  };

  /** Reads a cutter's own list for one aim, and answers with what was installed — `null` when the
   *  read failed or the aim moved past it, because a caller that re-derives the draft from the list
   *  must not do that from either. A failure is kept rather than only raised: the editor is withheld
   *  until a list arrives, so a read nobody reports leaves "reading" on screen for good. */
  const readPresets = (generation: number, machineId: string): Promise<Preset[] | null> =>
    ipc.listPresets(machineId).then(
      (p) => (installPresets(generation, machineId, p) ? (p as Preset[]) : null),
      (e) => {
        if (isCurrentAim(generation)) setPresetListError(ipc.ipcErrorMessage(e));
        return null;
      },
    );

  /** The connected cutter this dialog has already aimed presets at, so a store update caused by
   *  connect() does not load the list a second time. */
  const handledKey = useRef<string | null>(null);

  // The store is the source of the connected cutter, including one chosen from the pill while this
  // dialog is open and one that drops out. connect() claims the key first so this does not run for
  // the click that already loads caps and presets.
  const connectedKey = cutterKey(connected);
  useEffect(() => {
    if (handledKey.current === connectedKey) return;
    handledKey.current = connectedKey;
    if (!connected) {
      aimPresetsAt(null);
      return;
    }
    const machineId = connected.machine_id;
    const aim = aimPresetsAt(machineId);
    // Separate chains on purpose: a corrupt presets file must not leave caps unfetched, and an
    // unknown machine must not blank the preset dropdown.
    ipc
      .machineCaps(machineId)
      .then((c) => setCapsFor({ machineId, caps: c as Caps }))
      .catch((e) => onError(ipc.ipcErrorMessage(e)));
    void readPresets(aim, machineId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectedKey]);

  /** Marks the held list superseded, which every write does the moment it lands: the presets file
   *  no longer holds what this list says, and the read that will say what it does hold is out. The
   *  rows stop pricing passes from it here rather than when that read lands, because a read that
   *  fails does not land at all — and the pre-write name, speed, force and repeat would otherwise
   *  stand on the rows for the life of the dialog on this cutter (#274). */
  const supersedePresets = (generation: number) => {
    if (!isCurrentAim(generation)) return;
    setPresetsFor((prev) => (prev === null ? prev : { ...prev, current: false }));
  };

  const editPreset = (id: string) => {
    const stored = presets.find((p) => p.id === id);
    if (!stored) return;
    setDraft(draftOf(stored));
    setBaseline(draftOf(stored));
    setPresetError(null);
  };

  const newPreset = () => {
    if (ranges === null) return;
    setDraft(newDraft(ranges));
    setBaseline(newDraft(ranges));
    setPresetError(null);
  };

  /** The list is re-read from the backend after a write and the draft re-derived from it, the way
   *  the device list is: the file is what a preset is, and an entry patched in here would be this
   *  side's opinion of what was stored. `then` is whatever was waiting on this save.
   *
   *  A refusal keeps the draft exactly as typed and names itself beside it — a write the backend
   *  refused is the one moment those numbers exist nowhere else.
   *
   *  The write's own failure and the re-read's are held apart. Caught together, a read that failed
   *  after a write that did not reported a refused save, kept the draft as unsaved, and swallowed
   *  whatever was waiting on it (CodeRabbit on PR #264). */
  const writePreset = (preset: Preset, then?: () => void) => {
    // The aim this write belongs to. Everything below is checked against it: a write outlives a
    // cutter change, and so does whatever the operator asked for next.
    const aim = presetAim.current.generation;
    setPresetBusy(true);
    setPresetError(null);
    ipc
      .savePreset(preset)
      .then(
        () => {
          // Only while the section is still on the aim that was written to. `copyPreset` writes
          // without the unsaved-changes guard, so an operator can press Save as Copy and then aim
          // elsewhere before the reply lands — and a draft re-installed after `aimPresetsAt` cleared
          // it would be the previous aim's entry, shown against this one and saved under its
          // machine's id (CodeRabbit on PR #264).
          if (isCurrentAim(aim)) {
            // The write landed, so the file holds what was sent: the draft stops being unsaved here
            // rather than at the re-read, or a refresh that failed afterwards would leave the
            // operator holding an "unsaved" edit that is in fact saved — and every guard would keep
            // asking about it. The re-read refines this with what the backend actually lists.
            const written = draftOf(preset);
            setDraft(written);
            setBaseline(written);
          }
          supersedePresets(aim);
          return readPresets(aim, preset.machine_id).then((stored) => {
            const saved = stored?.find((p) => p.id === preset.id);
            if (saved !== undefined) {
              setDraft(draftOf(saved));
              setBaseline(draftOf(saved));
            }
            // Last, and whether or not that read landed: the continuation is what the operator
            // asked for *next* — another preset, a blank one, a close. Run before the re-read, its
            // selection was overwritten by this write's own (Codex on PR #264).
            //
            // And only under the aim it was captured on: `editPreset` closes over that aim's list,
            // so run after a cutter change it restored the previous cutter's entry as this one's
            // draft — which the next save wrote under this machine's id (Greptile on PR #264). An
            // aim change *is* a legal continuation: the aim has not moved yet when it runs.
            if (isCurrentAim(aim)) then?.();
          });
        },
        (e) => {
          // Named against the draft it refused, which is on screen only while the aim has not moved.
          // The continuation does not run: nothing was written to continue from.
          if (!isCurrentAim(aim)) return;
          setPresetError(ipc.ipcErrorMessage(e));
          clearPending();
        },
      )
      .finally(() => setPresetBusy(false));
  };

  const savePresetDraft = (then?: () => void) => {
    if (draft === null || connected === null || presetFault !== null) return;
    writePreset(toPreset(draft, connected.machine_id, presets), then);
  };

  /** A copy is written at once rather than opened as a draft: copying is the only way to edit what
   *  a builtin holds, and an unsaved copy would be a second unsaved thing to decide about. The id
   *  is minted fresh, so nothing about the source is rewritten or shadowed. */
  const copyPreset = () => {
    const source = draft === null ? undefined : presets.find((p) => p.id === draft.id);
    if (source === undefined || connected === null) return;
    writePreset(toPreset(copyDraft(source, presets), connected.machine_id, presets));
  };

  const deletePresetDraft = () => {
    if (draft === null || connected === null) return;
    const machineId = connected.machine_id;
    // Worked out before the delete, off the list that still holds the entry: afterwards there is
    // no row to take a neighbour from, and an editor showing a deleted preset's settings is the
    // one answer a delete must not give.
    const nextId = selectAfterDelete(presets, draft.id);
    const aim = presetAim.current.generation;
    setPresetBusy(true);
    setPresetError(null);
    ipc
      .deletePreset(machineId, draft.id)
      // Same split as `writePreset`: the delete happened, so a re-read that fails afterwards is a
      // read failure — reported as one, and never as a delete that left the entry in place.
      .then(
        () => {
          // The entry is gone, so it stops being what the editor holds here rather than at the
          // re-read: a refresh that failed afterwards would otherwise leave a deleted preset's
          // settings on screen. Past an aim change there is nothing of this aim's left to clear, and
          // the re-read below is what moves the selection to a neighbour.
          if (isCurrentAim(aim)) {
            setDraft(null);
            setBaseline(null);
          }
          supersedePresets(aim);
          return readPresets(aim, machineId);
        },
        (e) => {
          if (isCurrentAim(aim)) setPresetError(ipc.ipcErrorMessage(e));
          return null;
        },
      )
      .then((stored) => {
        if (stored === null) return;
        const next = stored.find((p) => p.id === nextId);
        setDraft(next === undefined ? null : draftOf(next));
        setBaseline(next === undefined ? null : draftOf(next));
      })
      .finally(() => setPresetBusy(false));
  };

  /** Back to the stored entry, and the refusal goes with it: a rejected write is about numbers that
   *  are no longer on screen, so leaving it up blames the entry the operator kept (Copilot on
   *  PR #264). */
  const discardDraft = () => {
    setDraft(baseline);
    setPresetError(null);
  };

  const connect = (info: ipc.DeviceInfo) => {
    // Marked before the request, not after it: the aim is already on its way to another cutter, and
    // an edit typed while it travels would be dropped by `aimPresetsAt` when it lands.
    // The key is claimed first so the effect that follows `connected` does not load the same list
    // again when the store publishes this cutter.
    setAiming(true);
    const previous = handledKey.current;
    handledKey.current = cutterKey(info);
    onConnect(info)
      .then((retargeted) => {
        // set_machine rewrites the document the open plan was hashed against. Replan before
        // Start Cut is reachable again (`aiming` holds it), or the first cut is refused stale.
        if (retargeted) replan();
        const aim = aimPresetsAt(info.machine_id);
        ipc
          .machineCaps(info.machine_id)
          .then((c) => setCapsFor({ machineId: info.machine_id, caps: c as Caps }))
          .catch((e) => onError(ipc.ipcErrorMessage(e)));
        return readPresets(aim, info.machine_id);
      })
      .catch((e) => {
        handledKey.current = previous;
        onError(ipc.ipcErrorMessage(e));
      })
      .finally(() => setAiming(false));
  };

  // The store drops the device. A row still labelled connected would offer no way back to Connect.
  const disconnect = () => {
    setAiming(true);
    const previous = handledKey.current;
    handledKey.current = "";
    onDisconnect()
      .then(() => {
        aimPresetsAt(null);
      })
      .catch((e) => {
        handledKey.current = previous;
        onError(ipc.ipcErrorMessage(e));
      })
      .finally(() => setAiming(false));
  };

  return {
    aiming,
    presetBusy,
    caps,
    presets,
    presetsLoaded,
    presetLookup,
    presetListError,
    draft,
    setDraft,
    presetMode,
    presetDirty,
    presetFault,
    presetError,
    connect,
    disconnect,
    editPreset,
    newPreset,
    copyPreset,
    deletePresetDraft,
    savePresetDraft,
    discardDraft,
  };
}
