// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import * as ipc from "../../ipc";
import {
  carryRows,
  presetIdForKey,
  readWeedDraft,
  reorderForReplan,
  reorderTo,
  toTravelPasses,
  weedDraftFrom,
  type PassVm,
  type WeedDraft,
} from "../viewmodel";

export type PassRow = PassVm & { nodeIds: number[]; starts: ([number, number] | null)[]; weed: [number, number][][] };

/** Before the ranges (and so the defaults) arrive: no weeding, with nothing typed. */
const NO_WEED: WeedDraft = { border: false, margin: "", lines: "None", spacing: "", clearance: "" };

const sameWeed = (a: ipc.WeedOptions | null, b: ipc.WeedOptions | null) => JSON.stringify(a) === JSON.stringify(b);

/** A plan the dialog is showing: the mode that produced it, the revision it was planned
 *  against, its rows, its skipped count, and the travel between those rows. One value because
 *  they are only ever true together — a mode change that left the previous rows sendable is how
 *  travel and a cut end up describing different geometry.
 *
 *  Travel lives here rather than beside this, because it is derived from the plan and only
 *  meaningful against these rows. Held separately it could be cleared while the plan it belonged
 *  to was still installed and cuttable, which is exactly what happened: a replan cleared travel on
 *  the way out, and a *second* replan then captured the cleared value as the one to restore. There
 *  is nothing to restore now — a failed replan does not touch the plan, so it does not touch its
 *  travel.
 */
export type InstalledPlan = {
  grouping: ipc.Grouping;
  /** The weed options the rows were planned with, held here for the same reason as the mode. */
  weed: ipc.WeedOptions | null;
  revision: string;
  rows: PassRow[];
  skippedNotCut: number;
  travel: [number, number, number, number][];
};

/** The installed plan, the grouping the operator has chosen, and the weed draft those rows were
 *  planned against. One hook because a mode change that left the previous rows sendable is how
 *  travel and a cut end up describing different geometry. */
export function useCutPlan({
  weed,
  onWeedChange,
  ranges,
  onError,
}: {
  weed: WeedDraft | null;
  onWeedChange: (draft: WeedDraft) => void;
  ranges: ipc.SettingsRanges | null;
  onError: (msg: string) => void;
}) {
  /** A plan and everything derived from it, installed as one value. The mode belongs here
   *  rather than beside it: rows keyed under one grouping must never be sent under another,
   *  and the stale-plan check guards the document, not the mode. */
  const [plan, setPlan] = useState<InstalledPlan | null>(null);
  /** The mode the operator has chosen, which is `plan.grouping` except while a replan is in
   *  flight. Cut and the row controls are unavailable in that window, because rows from the
   *  previous mode would otherwise be sendable under this one. */
  const [grouping, setGrouping] = useState<ipc.Grouping>("Color");
  const [replanning, setReplanning] = useState(false);
  const weedDraft: WeedDraft = weed ?? (ranges ? weedDraftFrom(ranges.weedDefaults, false) : NO_WEED);
  const weedRead = readWeedDraft(weedDraft, ranges?.weed ?? null);
  /** The weed on screen is the weed the rows were planned with. Cut waits for both: a draft that
   *  does not read must not cut without its border, and one that reads differently from the plan
   *  has not been previewed. */
  const weedSettled = weedRead.ok && sameWeed(weedRead.options, plan?.weed ?? null);
  const [stalePlan, setStalePlan] = useState(false);

  /** Serial number of the newest travel-affecting request; see movePass and replan. */
  const travelSeq = useRef(0);
  /** Serial number of the newest Replan, so two in flight cannot install out of order. */
  const planSeq = useRef(0);
  /** The plan as last committed, for a reply that lands after the rows were rearranged since its
   *  request went out: what it carries over must be the arrangement on screen now. Grouping and
   *  weed sit with it. Connect and Convert await before they replan, and the grouping select stays
   *  usable in that window — a function closed over the click would plan the mode the operator has
   *  since left. Written after commit, because a render React discards must not publish a grouping
   *  or a plan the operator never saw. A change made in the same event passes its new value into
   *  `replan` directly. */
  const planRef = useRef(plan);
  const groupingRef = useRef(grouping);
  const weedReadRef = useRef(weedRead);
  const weedDraftRef = useRef(weedDraft);
  useLayoutEffect(() => {
    planRef.current = plan;
    groupingRef.current = grouping;
    weedReadRef.current = weedRead;
    weedDraftRef.current = weedDraft;
  }, [plan, grouping, weedRead, weedDraft]);
  /** Counts weed edits, so a refused replan puts the controls back only if nothing was typed since
   *  it went out: typing that plans nothing (a cleared field) does not move `planSeq`. */
  const weedEdits = useRef(0);

  // Travel is the planner's answer about the configured list — reorder and enable both
  // change it, so both come through here. Rapid clicks fire several replans and nothing
  // orders their replies: an older response landing last would redraw travel for a list
  // the rows no longer show. Only the latest request's reply (or failure) may touch
  // state, and `replan` bumps the sequence too, so a fresh plan orphans them all.
  // `base` is the installed plan unless a replan passes the one it is installing this moment.
  const refreshTravel = (next: PassRow[], base: InstalledPlan | null = plan) => {
    // No plan means no travel on screen to go stale (the initial plan itself failed).
    if (base === null) return;
    const seq = ++travelSeq.current;
    ipc
      .travelForOrder(base.revision, base.grouping, base.weed, toTravelPasses(next))
      .then((t) => {
        // Onto whatever plan is installed when the reply lands, not the one captured when it was
        // asked for: the sequence guard has already established they are the same plan.
        if (seq === travelSeq.current) setPlan((prev) => (prev === null ? prev : { ...prev, travel: t }));
      })
      .catch((e) => {
        if (seq !== travelSeq.current) return;
        // The same refusal Start Cut gets, surfaced the same way: the banner's Replan
        // is the way back, and the rows keep the operator's arrangement.
        if (ipc.ipcErrorCode(e) === "stale_plan") setStalePlan(true);
        else onError(ipc.ipcErrorMessage(e));
      });
  };

  // A draft that does not read plans with the weed already installed, so Replan still works.
  // `keepRows` carries the operator's arrangement into the new rows (`carryRows`): a weed edit
  // changes no pass. Every other replan starts the rows afresh, as it always has.
  const replan = (mode?: ipc.Grouping, weedOptions?: ipc.WeedOptions | null, keepRows = false) => {
    const chosen = mode ?? groupingRef.current;
    const read = weedReadRef.current;
    const chosenWeed = weedOptions !== undefined ? weedOptions : read.ok ? read.options : planRef.current?.weed ?? null;
    const seq = ++planSeq.current;
    const editAt = weedEdits.current;
    const restore = planRef.current;
    setReplanning(true);
    // Nothing is cleared or orphaned on the way out. The previous plan stays installed until this
    // reply lands, and if the reply is a failure it stays installed for good — so both the travel
    // describing it and any travel reply still owed to it are still wanted. Orphaning them here is
    // what Codex caught: disable a pass, hold its (now zero-move) travel reply, then fail a
    // replan, and the plan keeps the edited rows while the reply that would have matched them was
    // thrown away. One enabled pass, one travel move, forever.
    //
    // Installation is where orphaning belongs, and it is enough: it happens at the one moment the
    // rows a pending reply was computed for stop being the rows on screen. A reply landing before
    // then was asked for against the plan that is still installed, so it is not stale at all.
    ipc
      .planCut(chosen, chosenWeed)
      .then((response) => {
        if (seq !== planSeq.current) return; // a newer Replan owns the dialog now
        // The rows are about to change, so every travel reply owed to the old ones is stale from
        // here: a move made while this fetch was out carries the old revision, and its late
        // stale_plan rejection would re-raise the banner this plan just cleared (Greptile drove
        // exactly that interleaving on PR #142).
        travelSeq.current++;
        const fresh: PassRow[] = response.passes.map((p) => ({
          key: p.key,
          shapeCount: p.shape_count,
          nodeIds: p.node_ids,
          starts: p.starts,
          weed: p.weed,
          enabled: true,
          // A preset-keyed pass starts with the preset it is keyed on, or it would be cut
          // with defaults — the one thing grouping by material exists to avoid.
          presetId: presetIdForKey(p.key),
          speed: null,
          force: null,
          repeatCount: null,
        }));
        const prev = planRef.current;
        const rows = keepRows && prev !== null && prev.grouping === chosen ? carryRows(prev.rows, fresh) : fresh;
        const installed: InstalledPlan = {
          grouping: chosen,
          weed: chosenWeed,
          revision: response.doc_revision,
          skippedNotCut: response.skipped_not_cut,
          rows,
          travel: response.travel,
        };
        setPlan(installed);
        setStalePlan(false);
        // The reply's travel is for every pass in planned order; carried rows that differ need
        // travel for the list as arranged, against the plan just installed.
        if (rows.some((r, i) => r.key !== fresh[i].key || !r.enabled)) refreshTravel(rows, installed);
      })
      .catch((e) => {
        if (seq !== planSeq.current) return; // superseded: its failure is no longer news
        // A plan that failed to install leaves the previous one in force, rows and mode alike,
        // so the picker goes back to *its* grouping. Left showing the mode nobody managed to
        // plan, the dialog would offer a Cut it cannot keep: the operator reads "one pass" and
        // the machine does the split the old plan still holds.
        setGrouping(restore?.grouping ?? "Color");
        // The weed controls go back to the installed plan's too, for the same reason.
        if (restore !== null && weedEdits.current === editAt) {
          onWeedChange(restore.weed ? weedDraftFrom(restore.weed, true) : { ...weedDraftRef.current, border: false });
        }
        onError(ipc.ipcErrorMessage(e));
      })
      .finally(() => {
        if (seq === planSeq.current) setReplanning(false);
      });
  };

  useEffect(() => {
    replan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Replanned at once, like a grouping change, whenever the draft reads: the preview has to show
  // the border before it can be cut. Every reading change replans, even back to the installed
  // weed, because an earlier replan may still be in flight and `planSeq` lets only the newest
  // install. A draft that does not read plans nothing, and Cut waits (`weedSettled`).
  const changeWeed = (next: WeedDraft) => {
    weedEdits.current++;
    onWeedChange(next);
    const read = readWeedDraft(next, ranges?.weed ?? null);
    if (read.ok) replan(grouping, read.options, true);
  };

  const updateRow = (i: number, patch: Partial<PassRow>) => {
    setPlan((prev) => (prev === null ? prev : {
      ...prev,
      rows: prev.rows.map((r, idx) => (idx === i ? { ...r, ...patch } : r)),
    }));
  };

  const movePass = (i: number, dir: -1 | 1) => {
    if (plan === null) return;
    const next = reorderForReplan(plan.rows, i, dir);
    if (!next) return;
    setPlan({ ...plan, rows: next });
    refreshTravel(next);
  };

  // A drag lands on an index, not a direction. The same travel request as a one-step move.
  const movePassTo = (from: number, to: number) => {
    if (plan === null) return;
    const next = reorderTo(plan.rows, from, to);
    if (!next) return;
    setPlan({ ...plan, rows: next });
    refreshTravel(next);
  };

  /** One material for every pass: the preset's own speed and force show, and a typed
   *  override would hide them. Clearing the choice puts the blanks back. */
  const applyMaterial = (presetId: string | null) => {
    setPlan((prev) => (prev === null ? prev : {
      ...prev,
      rows: prev.rows.map((r) => ({ ...r, presetId, speed: null, force: null, repeatCount: null })),
    }));
  };

  // The head never travels to a pass that will not be cut, so enabling is as much a
  // travel edit as reordering is.
  const setPassEnabled = (i: number, enabled: boolean) => {
    if (plan === null) return;
    const next = plan.rows.map((r, idx) => (idx === i ? { ...r, enabled } : r));
    setPlan({ ...plan, rows: next });
    refreshTravel(next);
  };

  return {
    plan,
    grouping,
    setGrouping,
    replanning,
    weedDraft,
    weedRead,
    weedSettled,
    stalePlan,
    setStalePlan,
    replan,
    changeWeed,
    updateRow,
    movePass,
    movePassTo,
    applyMaterial,
    setPassEnabled,
  };
}
