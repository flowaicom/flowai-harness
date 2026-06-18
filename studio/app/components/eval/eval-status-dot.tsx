/**
 * Eval status indicator dot.
 *
 * Thin wrapper over shared StatusDot with eval-specific color map.
 *
 * @module components/eval/eval-status-dot
 */

import { StatusDot } from "~/components/shared/status-dot";
import { EVAL_STATUS_COLORS, type EvalStatusKey } from "~/lib/domain/eval";

const PULSE_STATUSES: readonly EvalStatusKey[] = ["running", "paused"];

interface EvalStatusDotProps {
  status: EvalStatusKey;
  size?: number;
  pulse?: boolean;
  className?: string;
}

export function EvalStatusDot(props: EvalStatusDotProps) {
  return <StatusDot {...props} colorMap={EVAL_STATUS_COLORS} pulseStatuses={PULSE_STATUSES} />;
}
