/**
 * Re-run dialog for harness eval runs.
 *
 * The harness re-streams the full eval run; per-case rerun selection is not
 * supported yet.
 *
 * @module components/eval/rerun-dialog
 */

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "~/components/ui/dialog";
import type { TestCaseResult } from "~/lib/domain/eval";

interface RerunDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  results: readonly TestCaseResult[];
  passThreshold: number;
  onSubmit: (testCaseIds: string[]) => void;
}

export function RerunDialog({ open, onOpenChange, results, onSubmit }: RerunDialogProps) {
  const handleSubmit = () => {
    onSubmit(results.map((result) => result.testCaseId));
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Re-run Eval</DialogTitle>
          <DialogDescription>
            This will re-stream the full eval run with the same test cases.
          </DialogDescription>
        </DialogHeader>

        <div className="rounded-md border bg-muted/30 px-3 py-2 text-sm text-muted-foreground mt-4">
          {results.length} test case{results.length === 1 ? "" : "s"} will be included.
        </div>

        <div className="flex items-center justify-between mt-4">
          <span className="text-xs text-muted-foreground">Harness reruns all cases.</span>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => onOpenChange(false)}
              className="text-sm px-3 py-1.5 rounded-md border hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={handleSubmit}
              className="text-sm font-medium px-3 py-1.5 rounded-md bg-primary text-primary-foreground hover:bg-primary/90 disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            >
              Re-run Eval
            </button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
