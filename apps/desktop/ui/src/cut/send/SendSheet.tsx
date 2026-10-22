// SPDX-License-Identifier: GPL-3.0-or-later
import { useEffect, useState } from "react";
import * as ipc from "../../ipc";
import type { Scene } from "../../render/hittest";
import type { ResolvedTheme } from "../../theme";
import type { WeedDraft } from "../viewmodel";
import { btn, dialogStyle, panelStyle } from "./chrome";
import { AlreadyAcceptedNote, StepPreview } from "./StepPreview";
import { HostPairing, StepMachine, useDeviceList } from "./StepMachine";
import { StepPasses } from "./StepPasses";
import { useCutDispatch } from "./useCutDispatch";
import { useCutPlan } from "./useCutPlan";
import { usePresetsForAim } from "./usePresetsForAim";

/** Phase 4b's stepper writes this. Until then every section stays on screen. */
export type SendStep = 1 | 2 | 3;

function shownSections(step: SendStep): { machine: boolean; passes: boolean; preview: boolean } {
  switch (step) {
    case 1:
    case 2:
    case 3:
      // ponytail: one scroll until phase 4b's stepper. The step is already the shell's, so that
      // stepper is a writer rather than a new home for the state. Upgrade: return false for the
      // sections a step does not show.
      return { machine: true, passes: true, preview: true };
    default: {
      const unreachable: never = step;
      return unreachable;
    }
  }
}

type Props = {
  scene: Scene;
  artboard: { x: number; y: number; w: number; h: number };
  docMachineId: string | null;
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
  /** The weed controls as last left, kept by App for the session; null until first changed, when
   *  they start from the backend's defaults. */
  weed: WeedDraft | null;
  onWeedChange: (draft: WeedDraft) => void;
  /** App's resolved theme. A second `useTheme()` keeps its own state, so a keyboard
   *  appearance change while this dialog is open would leave the swatches on the old theme. */
  theme: ResolvedTheme;
};

export function SendSheet({
  scene,
  artboard,
  docMachineId,
  status,
  refreshDeviceState,
  onConvertMachine,
  connected,
  onConnect,
  onDisconnect,
  onReconnect,
  onError,
  onClose,
  weed,
  onWeedChange,
  theme,
}: Props) {
  const [step] = useState<SendStep>(1);
  const shown = shownSections(step);
  const device = useDeviceList({ refreshDeviceState, onError, onReconnect });

  /** The bounds a setting must sit in, from `cutplan::preflight` — the same module that refuses a
   *  cut over them. Held rather than defaulted: with no answer there is nothing to validate a
   *  preset against, and guessed bounds would offer saves the cut path then refuses. */
  const [ranges, setRanges] = useState<ipc.SettingsRanges | null>(null);
  const [rangesError, setRangesError] = useState<string | null>(null);
  useEffect(() => {
    // Its own chain, and its failure is kept rather than raised: it withholds the preset editor
    // (see below) instead of taking the dialog's other sections down with it.
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

  const dispatch = useCutDispatch({
    connected,
    plan: plan.plan,
    replanning: plan.replanning,
    weedSettled: plan.weedSettled,
    onStalePlan: () => plan.setStalePlan(true),
    onError,
    refreshDeviceState,
  });

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

  return (
    <>
    <div style={panelStyle}>
      <div role="dialog" aria-modal="true" aria-label="Cut" data-send-step={step} style={dialogStyle}>
        <div style={{ display: "flex", alignItems: "center" }}>
          <strong>Cut</strong>
          <div style={{ flex: 1 }} />
          <button aria-label="Close" style={btn} onClick={() => guardUnsaved(onClose)}>
            Close
          </button>
        </div>

        {shown.machine ? (
          <StepMachine
            devices={device.devices}
            hosts={device.hosts}
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
          />
        ) : null}

        {shown.passes ? (
          <StepPasses
            mid={
              <>
                {/* The decision is the operator's, and nothing moves until they make it: Save writes and
                    then does what they asked for, Discard drops the edit and does it, Keep editing leaves
                    them where they were. Save is withheld while the draft is refused, with the reason
                    already on screen in the editor above. */}
                {pendingAfterDecision !== null ? (
                  <div role="alert" style={{ color: "var(--cut)", fontSize: 12, display: "flex", alignItems: "center", gap: 8 }}>
                    <span>Unsaved changes to the preset “{presets.draft?.name}”.</span>
                    <button
                      aria-label="Save preset and continue"
                      style={btn}
                      disabled={presets.presetBusy || presets.presetFault !== null}
                      onClick={saveThenContinue}
                    >
                      Save and continue
                    </button>
                    <button aria-label="Discard preset changes and continue" style={btn} disabled={presets.presetBusy} onClick={discardThenContinue}>
                      Discard changes
                    </button>
                    <button aria-label="Keep editing the preset" style={btn} onClick={() => setPendingAfterDecision(null)}>
                      Keep editing
                    </button>
                  </div>
                ) : null}

                {machineMismatch && connected ? (
                  <div style={{ color: "var(--cut)", fontSize: 12, display: "flex", alignItems: "center", gap: 8 }}>
                    Document is set up for a different machine than the connected device.
                    <button
                      style={btn}
                      onClick={() => {
                        // The conversion is the document change. A refused set_machine leaves the open plan,
                        // which still matches the document (Bugbot on #313).
                        void Promise.resolve(onConvertMachine(connected.machine_id)).then((changed) => {
                          if (changed) plan.replan();
                        });
                      }}
                    >
                      Convert to {connected.machine_id}
                    </button>
                  </div>
                ) : null}

                <AlreadyAcceptedNote show={dispatch.alreadyAccepted} />

                {plan.stalePlan ? (
                  <div style={{ color: "var(--cut)", fontSize: 12, display: "flex", alignItems: "center", gap: 8 }}>
                    Document changed since this plan was made.
                    <button style={btn} onClick={() => plan.replan()}>
                      Replan
                    </button>
                  </div>
                ) : null}
              </>
            }
            connected={connected}
            ranges={ranges}
            rangesError={rangesError}
            weed={weed}
            theme={theme}
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
            planRows={plan.plan?.rows ?? null}
            planGrouping={plan.plan?.grouping ?? null}
            skippedNotCut={plan.plan?.skippedNotCut ?? null}
            presetLookup={presets.presetLookup}
            updateRow={plan.updateRow}
            setPassEnabled={plan.setPassEnabled}
            movePass={plan.movePass}
          />
        ) : null}

        {shown.preview ? (
          <StepPreview
            scene={scene}
            artboard={artboard}
            passes={plan.plan?.rows ?? []}
            travel={plan.plan?.travel ?? []}
            theme={theme}
          />
        ) : null}

        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {status.phase === "AwaitingColorSwap" ? <span>Waiting for color swap</span> : null}
          {status.phase === "Sending" && status.sent ? (
            <span>
              sending {status.sent.sent} / {status.sent.total} bytes
            </span>
          ) : null}
          {status.phase === "Cancelling" ? <span>cancelling</span> : null}
          {status.phase === "AwaitingConfirmation" ? <span>Awaiting completion</span> : null}
          {/* How the last job ended comes from the backend, so the dialog remembers
              nothing: no latch, no reading `actions.cancel` as a liveness bit. */}
          {status.ended === "Cancelled" ? <span style={{ color: "var(--muted-foreground)" }}>Cancelled</span> : null}
          {status.ended === "Completed" ? <span>Job complete</span> : null}
          {status.phase === "Failed" ? <span style={{ color: "var(--cut)" }}>Cut failed</span> : null}

          <div style={{ flex: 1 }} />

          {status.actions.resume ? (
            <button aria-label="Resume" style={btn} onClick={dispatch.resume}>
              Resume
            </button>
          ) : null}
          {status.actions.confirm ? (
            <button aria-label="Confirm pass done" style={btn} onClick={dispatch.confirmPassDone}>
              Confirm pass done
            </button>
          ) : null}
          {status.actions.cancel ? (
            <button aria-label="Cancel" style={btn} onClick={dispatch.cancel}>
              Cancel
            </button>
          ) : null}
          {/* The only enablement checks left are the ones the backend cannot know:
              whether this dialog has a device, a matching machine and rows to cut. */}
          <button
            aria-label="Start Cut"
            style={btn}
            disabled={!status.actions.cut || !connected || machineMismatch || plan.plan === null
              || plan.plan.rows.length === 0 || plan.replanning || dispatch.cutInFlight || !plan.weedSettled}
            onClick={dispatch.startCut}
          >
            Start Cut
          </button>
        </div>
      </div>
    </div>
    <HostPairing open={device.pairing} onPaired={device.paired} onClose={() => device.setPairing(false)} />
    </>
  );
}
