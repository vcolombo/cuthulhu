// SPDX-License-Identifier: GPL-3.0-or-later
import { Button } from "../../components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "../../components/ui/alert-dialog";

/** Confirm before a cancel. The destructive action is text on the card, not a filled
 *  button: dark `--destructive` on `--secondary` is under 4.5:1, and white on it is worse. */
export function CancelCutDialog({
  open,
  offline,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  offline: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Cancel this cut?</AlertDialogTitle>
          <AlertDialogDescription>
            The cutter stops after the command reaches it.
            {offline ? " It will be sent once the host answers." : ""}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel asChild>
            <Button variant="ghost">Keep cutting</Button>
          </AlertDialogCancel>
          <AlertDialogAction asChild>
            <Button variant="ghost" className="border border-destructive text-destructive" onClick={onConfirm}>
              Cancel the cut
            </Button>
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
