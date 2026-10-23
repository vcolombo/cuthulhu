// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, Check, Scissors, X } from "lucide-react";
import type { MachineProfile } from "../../App";
import * as ipc from "../../ipc";
import type { LengthUnit } from "../../length";
import type { Scene } from "../../render/hittest";
import type { ResolvedTheme } from "../../theme";
import { Button } from "../../components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "../../components/ui/sheet";
import { cn } from "../../lib/cn";
import type { WeedDraft } from "../viewmodel";
import { collectPassIssues } from "./passFields";
import { AlreadyAcceptedNote, StepPreview } from "./StepPreview";
import { HostPairing, StepMachine, useDeviceList } from "./StepMachine";
import { StepPasses } from "./StepPasses";
import type { JobSnapshot } from "../job/model";
import { useCutDispatch } from "./useCutDispatch";
import { useCutPlan } from "./useCutPlan";
import { usePresetsForAim } from "./usePresetsForAim";

const STEPS = ["Machine", "Material & passes", "Preview & cut"] as const;
export type SendStep = 1 | 2 | 3;

type Props = {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  docMachineId: string | null;
  machines: MachineProfile[];
  status: ipc.CutStatus;
  refreshDeviceState: () => Promise<void>;
  onConvertMachine: (machineId: string) => boolean | Promise<boolean>;
  /** The connected cutter, from the one store the pill and the status bar read. */
  connected: ipc.DeviceInfo | null;
  /** Resolves true when the document was retargeted to this cutter, which stales an open plan. */
  onConnect: (info: ipc.DeviceInfo) => Promise<boolean>;
  onDisconnect: () => Promise<unknown>;
  onReconnect: () => Promise<unknown>;
  onError: (msg: string) => void;
  onClose: () => void;
  /** A cut the host accepted. The sheet closes; the passes stay with the job panel. */
  onJobAccepted: (snapshot: JobSnapshot) => void;
  /** The weed controls as last left, kept by App for the session; null until first changed, when
   *  they start from the backend's defaults. */
  weed: WeedDraft | null;
  onWeedChange: (draft: WeedDraft) => void;
  /** App's resolved theme. A second `useTheme()` keeps its own state, so a keyboard
   *  appearance change while this dialog is open would leave the swatches on the old theme. */
  theme: ResolvedTheme;
  unit: LengthUnit;
};

export function SendSheet({
  scene,
  artboard,
  docMachineId,
  machines,
  status,
  refreshDeviceState,
  onConvertMachine,
  connected,
  onConnect,
  onDisconnect,
  onReconnect,
  onError,
  onClose,
  onJobAccepted,
  weed,
  onWeedChange,
  theme,
  unit,
}: Props) {
  const [step, setStep] = useState<SendStep>(1);
  // The first visit to a step is Next, which is what requires a cutter. Going back to the machine
  // list to disconnect must not trap the passes there: the row is where a dropped aim shows that
  // a material's name is unread, and Next will not leave step 1 without a cutter that can cut.
  const [reached, setReached] = useState<SendStep>(1);
  const [editorOpen, setEditorOpen] = useState(false);
  const headingRef = useRef<HTMLHeadingElement>(null);
  // Compared, not cleared: StrictMode runs this effect twice on mount and a flag flipped on
  // the first run focuses the heading as the sheet opens, ahead of the dialog's own focus.
  const stepSeen = useRef(step);
  const [weedEditing, setWeedEditing] = useState(false);
  const device = useDeviceList({ refreshDeviceState, onError, onReconnect });

  /** The bounds a setting must sit in, from `cutplan::preflight` — the same module that refuses a
   *  cut over them. Held rather than defaulted: with no answer there is nothing to validate a
   *  preset against, and guessed bounds would offer saves the cut path then refuses. */
  const [ranges, setRanges] = useState<ipc.SettingsRanges | null>(null);
  const [rangesError, setRangesError] = useState<string | null>(null);
  useEffect(() => {
    ipc
      .settingsRanges()
      .then(setRanges)
      .catch((e) => setRangesError(ipc.ipcErrorMessage(e)));
  }, []);

  const plan = useCutPlan({ weed, onWeedChange, ranges, onError });

  /** What the operator asked for while a draft was unsaved, held until they decide. Selecting
   *  another preset, changing cutter and closing the dialog all replace the draft, so each one
   *  arrives here first rather than discarding an edit on the way past. */
  const [pendingAfterDecision, setPendingAfterDecision] = useState<{ run: () => void } | null>(null);

  const presets = usePresetsForAim({
    connected,
    ranges,
    onError,
    onConnect,
    onDisconnect,
    replan: () => plan.replan(),
    clearPending: () => setPendingAfterDecision(null),
  });

  const rows = plan.plan?.rows ?? [];
  const issues = collectPassIssues(rows, presets.presetLookup, presets.caps, ranges);
  const dispatch = useCutDispatch({
    connected,
    plan: plan.plan,
    replanning: plan.replanning,
    weedSettled: plan.weedSettled,
    // Blank speed and force, and a plan the document has since left, never leave the sheet.
    // The button is disabled for the same reason; this still holds if the click arrives.
    canSend: () => !plan.stalePlan && collectPassIssues(plan.plan?.rows ?? [], presets.presetLookup, presets.caps, ranges).length === 0,
    onStalePlan: () => plan.setStalePlan(true),
    onError,
    presetLookup: presets.presetLookup,
    onAccepted: (snapshot) => {
      onJobAccepted(snapshot);
      onClose();
    },
  });

  // Moving between steps, not the open itself: the dialog's own focus lands on the first control.
  useEffect(() => {
    if (stepSeen.current === step) return;
    stepSeen.current = step;
    headingRef.current?.focus();
  }, [step]);

  /** Every action that replaces or drops the draft goes through here: selecting another preset,
   *  aiming at another cutter, and closing the dialog. An operator who typed a force and pressed
   *  Close deserves to be asked, and there is nowhere else those numbers exist. */
  const guardUnsaved = (run: () => void) => {
    if (presets.presetDirty) setPendingAfterDecision({ run });
    else run();
  };

  const saveThenContinue = () => {
    const waiting = pendingAfterDecision;
    setPendingAfterDecision(null);
    presets.savePresetDraft(() => waiting?.run());
  };

  const discardThenContinue = () => {
    const waiting = pendingAfterDecision;
    setPendingAfterDecision(null);
    presets.discardDraft();
    waiting?.run();
  };

  const machineMismatch = docMachineId !== null && connected !== null && docMachineId !== connected.machine_id;
  // A preset list still in flight does not hold the steps. `aiming` covers that read, and the
  // rows are what say the name is unread while it is out; Start waits on `replanning` instead,
  // which is the window a retarget's new plan has not installed yet.
  const readyCutter = connected !== null && status.actions.cut;
  const passesReady = plan.plan !== null && rows.length > 0 && !plan.replanning && plan.weedSettled
    && !weedEditing && issues.length === 0 && !machineMismatch && readyCutter && !dispatch.cutInFlight;
  // The first Start is what discovers a stale plan. Only the ones after the refusal wait.
  const startReady = passesReady && !plan.stalePlan;

  const go = (next: SendStep) => {
    setStep(next);
    setReached((prev) => (next > prev ? next : prev));
  };

  return (
    <>
      <Sheet open onOpenChange={(next) => { if (!next) guardUnsaved(onClose); }}>
        <SheetContent
          data-send-step={step}
          className="gap-0 p-0 sm:max-w-none"
          onInteractOutside={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
        >
          <div className="border-b border-border px-6 pb-4 pt-5">
            <div className="flex items-start gap-3">
              <div className="min-w-0 flex-1">
                <SheetTitle className="text-xl font-semibold">Send to cutter</SheetTitle>
                <SheetDescription className="mt-1 text-sm text-muted-foreground">
                  Three quick steps. Nothing is cut until you press Start cut.
                </SheetDescription>
              </div>
              <button type="button" aria-label="Close" data-ink="muted" className="rounded-md p-1 text-muted-foreground hover:bg-accent hover:text-foreground" onClick={() => guardUnsaved(onClose)}>
                <X className="size-4" aria-hidden />
              </button>
            </div>
            <ol className="mt-4 flex items-center gap-2" aria-label="Steps">
              {STEPS.map((title, index) => {
                const number = (index + 1) as SendStep;
                const current = number === step;
                const done = number < step;
                const opened = number <= reached;
                const badge = (
                  <span className={cn(
                    "grid size-6 place-items-center rounded-full border text-xs font-semibold",
                    current && "border-accent-ink text-accent-ink",
                    done && "border-success bg-success/15 text-success",
                    !current && !done && "border-border text-muted-foreground",
                  )}>
                    {done ? <Check className="size-3.5" aria-hidden /> : number}
                  </span>
                );
                return (
                  <li key={title} className="flex flex-1 items-center gap-2">
                    {current ? (
                      <h2 ref={headingRef} tabIndex={-1} aria-current="step" className="flex items-center gap-2 text-sm font-medium outline-none">
                        {badge}
                        {title}
                      </h2>
                    ) : (
                      <button
                        type="button"
                        disabled={!opened}
                        data-ink={opened ? "faded" : "muted"}
                        onClick={() => go(number)}
                        className={cn("flex items-center gap-2 text-sm", opened ? "text-foreground/80 hover:text-foreground" : "text-muted-foreground")}
                      >
                        {badge}
                        {title}
                      </button>
                    )}
                    {index < STEPS.length - 1 ? <span className="h-px flex-1 bg-border" /> : null}
                  </li>
                );
              })}
            </ol>
          </div>

          <div className="flex-1 space-y-4 overflow-auto px-6 py-5">
            {pendingAfterDecision !== null ? (
              <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-destructive">
                <span>Unsaved changes to the preset “{presets.draft?.name}”.</span>
                <Button variant="secondary" size="sm" aria-label="Save preset and continue" disabled={presets.presetBusy || presets.presetFault !== null} onClick={saveThenContinue}>
                  Save and continue
                </Button>
                <Button variant="secondary" size="sm" aria-label="Discard preset changes and continue" disabled={presets.presetBusy} onClick={discardThenContinue}>
                  Discard changes
                </Button>
                <Button variant="ghost" size="sm" aria-label="Keep editing the preset" onClick={() => setPendingAfterDecision(null)}>
                  Keep editing
                </Button>
              </div>
            ) : null}

            {machineMismatch && connected ? (
              <div className="flex flex-wrap items-center gap-2 text-sm text-destructive">
                Document is set up for a different machine than the connected device.
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => {
                    // The conversion is the document change. A refused set_machine leaves the open plan,
                    // which still matches the document (Bugbot on #313).
                    void Promise.resolve(onConvertMachine(connected.machine_id)).then((changed) => {
                      if (changed) plan.replan();
                    });
                  }}
                >
                  Convert to {connected.machine_id}
                </Button>
              </div>
            ) : null}

            <AlreadyAcceptedNote show={dispatch.alreadyAccepted} />

            {plan.stalePlan ? (
              <div className="flex flex-wrap items-center gap-2 text-sm text-destructive">
                Your design changed.
                <Button variant="secondary" size="sm" onClick={() => plan.replan()}>Replan</Button>
              </div>
            ) : null}

            {step === 1 ? (
              <StepMachine
                devices={device.devices}
                hosts={device.hosts}
                machines={machines}
                shown={device.shown}
                forceHost={device.forceHost}
                setForceHost={device.setForceHost}
                connected={connected}
                status={status}
                aiming={presets.aiming}
                guardUnsaved={guardUnsaved}
                onConnect={presets.connect}
                onDisconnect={presets.disconnect}
                onReconnect={device.reconnect}
                onForget={device.forget}
                onAddHost={() => device.setPairing(true)}
                onRetry={() => void device.refreshList()}
              />
            ) : null}

            {step === 2 ? (
              <StepPasses
                connected={connected}
                ranges={ranges}
                rangesError={rangesError}
                weed={weed}
                unit={unit}
                theme={theme}
                scene={scene}
                artboard={artboard}
                presets={presets.presets}
                presetsLoaded={presets.presetsLoaded}
                presetListError={presets.presetListError}
                caps={presets.caps}
                draft={presets.draft}
                presetMode={presets.presetMode}
                presetDirty={presets.presetDirty}
                presetFault={presets.presetFault}
                presetError={presets.presetError}
                presetBusy={presets.presetBusy}
                aiming={presets.aiming}
                editorOpen={editorOpen}
                onToggleEditor={() => setEditorOpen((open) => !open)}
                guardUnsaved={guardUnsaved}
                onSelectPreset={presets.editPreset}
                onNewPreset={presets.newPreset}
                onCopyPreset={presets.copyPreset}
                onChangeDraft={(patch) => presets.setDraft((prev) => (prev === null ? prev : { ...prev, ...patch }))}
                onSavePreset={() => presets.savePresetDraft()}
                onDiscardPreset={presets.discardDraft}
                onDeletePreset={presets.deletePresetDraft}
                grouping={plan.grouping}
                setGrouping={plan.setGrouping}
                replanning={plan.replanning}
                replan={plan.replan}
                weedDraft={plan.weedDraft}
                weedRead={plan.weedRead}
                changeWeed={plan.changeWeed}
                onWeedEditing={setWeedEditing}
                planRows={plan.plan?.rows ?? null}
                planGrouping={plan.plan?.grouping ?? null}
                skippedNotCut={plan.plan?.skippedNotCut ?? null}
                presetLookup={presets.presetLookup}
                issues={issues}
                updateRow={plan.updateRow}
                setPassEnabled={plan.setPassEnabled}
                movePass={plan.movePass}
                movePassTo={plan.movePassTo}
                onApplyMaterial={plan.applyMaterial}
              />
            ) : null}

            {step === 3 ? (
              <StepPreview
                scene={scene}
                artboard={artboard}
                passes={rows}
                travel={plan.plan?.travel ?? []}
                theme={theme}
                connected={connected}
                hosts={device.hosts}
                machines={machines}
                presetLookup={presets.presetLookup}
                grouping={plan.plan?.grouping ?? plan.grouping}
                weedOn={plan.weedDraft.border}
              />
            ) : null}
          </div>

          <div className="flex items-center gap-2 border-t border-border px-6 py-3">
            {step === 1 ? (
              <Button variant="ghost" onClick={() => guardUnsaved(onClose)}>Cancel</Button>
            ) : (
              <Button variant="ghost" onClick={() => go((step - 1) as SendStep)}>
                <ArrowLeft aria-hidden /> Back
              </Button>
            )}
            <div className="flex min-w-0 flex-1 flex-wrap items-center justify-end gap-x-3 gap-y-1 text-xs text-muted-foreground">
              {step === 1 && !readyCutter ? <span>Pick a cutter that's ready.</span> : null}
              {step === 2 && issues.length > 0 ? (
                <span role="alert" className="text-destructive">
                  Fix {issues.length} highlighted {issues.length === 1 ? "setting" : "settings"} to continue.
                </span>
              ) : null}
            </div>
            {step < 3 ? (
              <Button disabled={step === 1 ? !readyCutter : !passesReady} onClick={() => go((step + 1) as SendStep)}>
                Next <ArrowRight aria-hidden />
              </Button>
            ) : (
              <Button disabled={!startReady} onClick={dispatch.startCut}>
                <Scissors aria-hidden /> Start cut
              </Button>
            )}
          </div>
          <HostPairing open={device.pairing} onPaired={device.paired} onClose={() => device.setPairing(false)} />
        </SheetContent>
      </Sheet>
    </>
  );
}
