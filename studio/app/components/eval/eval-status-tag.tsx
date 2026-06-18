/**
 * Eval status tag.
 *
 * Thin wrapper over shared StatusTag with eval-specific color + label maps.
 *
 * @module components/eval/eval-status-tag
 */

import { StatusTag } from "~/components/shared/status-tag";
import { EVAL_STATUS_COLORS, type EvalStatusKey } from "~/lib/domain/eval";

const STATUS_LABELS: Record<EvalStatusKey, string> = {
  queued: "Queued",
  running: "Running",
  paused: "Paused",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
  skipped: "Skipped",
};

interface EvalStatusTagProps {
  status: EvalStatusKey;
  label?: string;
  className?: string;
}

export function EvalStatusTag(props: EvalStatusTagProps) {
  return <StatusTag {...props} colorMap={EVAL_STATUS_COLORS} labelMap={STATUS_LABELS} />;
}
