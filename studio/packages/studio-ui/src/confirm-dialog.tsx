import { AlertTriangleIcon, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Dialog, DialogBody, DialogFooter, DialogHeader } from "./dialog";
import { Button } from "./primitives";

export interface ConfirmDialogProps {
  readonly open: boolean;
  readonly title: ReactNode;
  readonly description?: ReactNode;
  readonly confirmLabel?: string;
  readonly cancelLabel?: string;
  readonly tone?: "default" | "destructive";
  readonly icon?: LucideIcon;
  readonly body?: ReactNode;
  readonly busy?: boolean;
  readonly onConfirm: () => void;
  readonly onClose: () => void;
}

export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  tone = "default",
  icon: Icon = AlertTriangleIcon,
  body,
  busy = false,
  onConfirm,
  onClose,
}: ConfirmDialogProps) {
  return (
    <Dialog open={open} onClose={onClose} labelledBy="confirm-dialog-title" size="sm">
      <DialogHeader
        id="confirm-dialog-title"
        title={title}
        description={description}
        icon={<Icon className="size-4" />}
        onClose={onClose}
      />
      {body ? <DialogBody>{body}</DialogBody> : null}
      <DialogFooter>
        <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
          {cancelLabel}
        </Button>
        <Button
          variant={tone === "destructive" ? "danger" : "primary"}
          size="sm"
          onClick={onConfirm}
          disabled={busy}
        >
          {confirmLabel}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
