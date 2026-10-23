// SPDX-License-Identifier: GPL-3.0-or-later
import { useRef } from "react";
import { Button } from "../components/ui/button";

type Props = {
  onImportFile: (file: File) => void;
  onTrace: () => void;
  onAddRect: () => void;
  /** Why the actions are off for now, or null; see `CanvasInteraction.editsLock`. */
  editsLocked: string | null;
};

/**
 * Shown whenever the document has no shapes. The card itself ignores the pointer: the rectangle
 * tool's tests click the middle of the mat, which is where this card sits, and that click has to
 * reach the canvas. Only the actions take the pointer.
 */
export function EmptyCanvas({ onImportFile, onTrace, onAddRect, editsLocked }: Props) {
  const fileRef = useRef<HTMLInputElement>(null);
  return (
    <div className="pointer-events-none absolute inset-0 z-[1] flex items-center justify-center p-6">
      <div className="pointer-events-none w-full max-w-xs rounded-md border border-dashed border-border bg-panel px-5 py-4 text-center shadow-sm">
        <h2 className="text-sm font-medium text-foreground">Your mat is empty</h2>
        <p className="mt-1 text-xs text-muted-foreground">Import a design, trace an image, or draw a shape.</p>
        <div className="pointer-events-auto mt-3 flex flex-wrap items-center justify-center gap-2">
          <Button variant="secondary" size="sm" disabled={editsLocked !== null} title={editsLocked ?? undefined} onClick={() => fileRef.current?.click()}>
            Import SVG
          </Button>
          <Button variant="secondary" size="sm" disabled={editsLocked !== null} title={editsLocked ?? undefined} onClick={onTrace}>
            Trace an image
          </Button>
          <Button variant="secondary" size="sm" disabled={editsLocked !== null} title={editsLocked ?? undefined} onClick={onAddRect}>
            Draw a rectangle
          </Button>
          <input
            ref={fileRef}
            type="file"
            accept=".svg,image/svg+xml"
            className="hidden"
            onChange={(event) => {
              const file = event.target.files?.[0];
              event.target.value = "";
              if (file) onImportFile(file);
            }}
          />
        </div>
      </div>
    </div>
  );
}
