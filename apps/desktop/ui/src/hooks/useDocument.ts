// SPDX-License-Identifier: GPL-3.0-or-later
import { useCallback, useEffect, useRef, useState, type SetStateAction } from "react";
import { toast } from "sonner";
import type { DocSnapshot, MachineProfile } from "../App";
import type { Preset } from "../cut/viewmodel";
import type { EditsLock } from "../interaction/useCanvasInteraction";
import * as ipc from "../ipc";
import type { HistoryFlags } from "../ipc";
import { toastCopy } from "./toastCopy";

const NO_HISTORY: HistoryFlags = { can_undo: false, can_redo: false };

/**
 * Document, selection, and the edit lock's half of the handshake. The canvas owns the other half
 * (`editsLockNow`); it is read through a ref because the two hooks call each other.
 */
export function useDocument(editsLockNow: { current: () => EditsLock }) {
  const [doc, setDoc] = useState<DocSnapshot | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [machines, setMachines] = useState<MachineProfile[]>([]);
  const [presets, setPresets] = useState<Preset[]>([]);
  // False until `listPresets` has answered for the machine on the document. A list still in
  // flight, a failed read, and no machine are the same fact: the names are not available.
  const [presetsLoaded, setPresetsLoaded] = useState(false);
  const [error, setErrorState] = useState<string | null>(null);
  // When the message on screen was raised, on the clock the canvas stamps its requests with: a
  // commit clears only a message older than the request it carries (Copilot on #301).
  const errorAt = useRef(0);
  const errorRef = useRef<string | null>(null);
  const setError = useCallback((next: SetStateAction<string | null>) => {
    const value = typeof next === "function" ? next(errorRef.current) : next;
    // A string argument is a new report. A function is a retraction, and must not look newer than
    // the commit that asked for it.
    if (typeof next === "string") errorAt.current = performance.now();
    const prev = errorRef.current;
    errorRef.current = value;
    setErrorState(value);
    // Toasts are a side effect of the value, not of the updater: StrictMode invokes updaters twice.
    if (value === prev) return;
    if (prev) toast.dismiss(prev);
    if (value) {
      const shown = toastCopy(value);
      toast.error(shown, {
        id: value,
        duration: 12000,
        action: {
          label: "Copy details",
          onClick: () => {
            void navigator.clipboard?.writeText(shown);
          },
        },
      });
    }
  }, []);
  const [lastPath, setLastPath] = useState<string | null>(null);
  const [readFailed, setReadFailed] = useState(false);
  const [history, setHistory] = useState<HistoryFlags>(NO_HISTORY);

  const revCounter = useRef(0);
  const [docRev, setDocRev] = useState(0);
  // Bumped when a load replaces the document. A snapshot asked for before that answers with the
  // document that left; rendered after the load, it showed that one under the next revision, which
  // also lifted the edit lock over the loaded document (Copilot on #301). So it is dropped, and
  // null says nothing rendered.
  const docGen = useRef(0);
  // Snapshots in the order they were asked for, and the latest that rendered. One answering after a
  // newer one rendered is older than what is on screen: shown under the next revision, it put the
  // canvas back to before an edit the newer one already showed (CodeRabbit on #301). It answers
  // with the revision on screen instead, which was read later and so holds everything it would
  // have: as null, a transform's preview stayed up over that newer scene (Copilot on #301).
  const snapshotsAsked = useRef(0);
  const snapshotShown = useRef(0);

  const refresh = useCallback(async (): Promise<number | null> => {
    const gen = docGen.current;
    const asked = ++snapshotsAsked.current;
    let json: string;
    try {
      json = (await ipc.snapshot()) as string;
    } catch (e) {
      if (gen === docGen.current) setReadFailed(true);
      throw e;
    }
    if (gen !== docGen.current) return null;
    if (asked < snapshotShown.current) return revCounter.current;
    snapshotShown.current = asked;
    const parsed = JSON.parse(json) as DocSnapshot;
    const rev = ++revCounter.current;
    setDoc(parsed);
    setDocRev(rev);
    setReadFailed(false);
    // The stacks are the editor's, not the document's. A failure here leaves the last flags: the
    // snapshot is what the canvas needs, and a missing flag must not lock the document (#308).
    try {
      const flags = await ipc.history();
      // The reply is the stacks at the moment it ran, which may be an earlier refresh than the
      // snapshot now on screen. Installing it would put Undo and Redo back (Bugbot on #313).
      if (gen === docGen.current && asked === snapshotShown.current) setHistory(flags);
    } catch {
      /* keep the previous flags */
    }
    return rev;
  }, []);

  // ponytail: every command re-fetches the full snapshot instead of applying its returned
  // Delta locally with reconcile() — correct and simple while scenes stay tiny. Canvas gestures
  // (useCanvasInteraction) use applyOptimistic for live feedback, then also just re-fetch once
  // their commit lands; reconcile() stays unused until per-frame delta application is worth it.
  const run = useCallback(
    async (fn: () => Promise<unknown>) => {
      try {
        setError(null);
        await fn();
        await refresh();
        return true;
      } catch (e) {
        // No silent failures: every caught error is surfaced. Device/cut commands reject with a
        // structured IpcError, not a string — ipcErrorMessage handles both that and the
        // plain-string rejections from the older doc-editing commands.
        setError(ipc.ipcErrorMessage(e));
        return false;
      }
    },
    [refresh, setError],
  );

  const editsStarted = useRef(new Set<Promise<boolean>>());
  const lockReason = useCallback(
    (lock: EditsLock) =>
      lock === null ? null
      : lock === "unread" && readFailed ? "the loaded document could not be read. Reload to try again"
      : "waiting for the document to load",
    [readFailed],
  );
  // A refusal after the loaded document's read failed also reads it again: the lock lifts once a
  // snapshot renders, so the next try can go through without a Reload.
  const refuseEdit = useCallback(
    (lock: "loading" | "unread") => {
      if (lock === "loading" || !readFailed) {
        setError("Not applied: the document is still loading");
        return;
      }
      const refused = "Not applied: the loaded document could not be read. Reading it again";
      setError(refused);
      refresh().then(
        (rev) => {
          // Null: a newer load replaced the document meanwhile, and its own read speaks for it.
          if (rev === null) return;
          setError((shown) => (shown === refused ? "Not applied, but the loaded document is on screen now: try again" : shown));
        },
        (e) => {
          const failed = `Not applied: the loaded document could not be read: ${ipc.ipcErrorMessage(e)}. Reload to try again`;
          setError((shown) => (shown === refused ? failed : shown));
        },
      );
    },
    [readFailed, refresh, setError],
  );
  const edit = useCallback(
    (fn: () => Promise<unknown>) => {
      const lock = editsLockNow.current();
      if (lock !== null) {
        refuseEdit(lock);
        return Promise.resolve(false);
      }
      const started = run(fn);
      editsStarted.current.add(started);
      void started.finally(() => editsStarted.current.delete(started));
      return started;
    },
    [run, refuseEdit, editsLockNow],
  );

  const waitForEdits = useCallback(() => Promise.all(editsStarted.current), []);
  // After loadProject, inside the canvas's replace lock: a snapshot asked for before the load
  // answers with the document that left (Copilot on #301).
  const noteLoaded = useCallback(() => {
    docGen.current++;
    setReadFailed(false);
  }, []);

  useEffect(() => {
    refresh().catch((e) => setError(ipc.ipcErrorMessage(e)));
    ipc
      .listMachines()
      .then((m) => setMachines(m as MachineProfile[]))
      .catch((e) => setError(ipc.ipcErrorMessage(e)));
  }, [refresh, setError]);

  // Re-read when the document's machine changes, because presets are machine-scoped: a project
  // converted from a Cameo to a Puma must not keep offering Cameo materials. No machine set
  // means no list — there is nothing for a preset id to be scoped to yet.
  const docMachineId = doc?.machine?.id ?? null;
  useEffect(() => {
    if (docMachineId === null) {
      setPresets([]);
      setPresetsLoaded(false);
      return;
    }
    // The previous machine's list is not an answer for this one. Clear it before the read,
    // or a slow reply would name the old materials and `loaded` would still say they were current.
    let cancelled = false;
    setPresets([]);
    setPresetsLoaded(false);
    ipc
      .listPresets(docMachineId)
      .then((p) => {
        if (cancelled) return;
        setPresets(p as Preset[]);
        setPresetsLoaded(true);
      })
      .catch((e) => {
        if (cancelled) return;
        setPresetsLoaded(false);
        setError(ipc.ipcErrorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [docMachineId, setError]);

  return {
    doc,
    selected,
    setSelected,
    machines,
    presets,
    presetsLoaded,
    error,
    setError,
    errorAt,
    lastPath,
    setLastPath,
    readFailed,
    docRev,
    history,
    refresh,
    run,
    edit,
    lockReason,
    waitForEdits,
    noteLoaded,
    root: doc?.root ?? 0,
  };
}
