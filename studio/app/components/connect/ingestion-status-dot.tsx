/**
 * Ingestion status indicator dot.
 *
 * Thin wrapper over shared StatusDot with ingestion-specific color map.
 *
 * @module components/data/ingestion-status-dot
 */

import { StatusDot } from "~/components/shared/status-dot";
import { INGESTION_STATUS_COLORS, type IngestionStatusKey } from "~/lib/domain/data";

const ACTIVE_STATUSES: readonly IngestionStatusKey[] = [
  "discovering",
  "profiling",
  "enriching",
  "extracting",
  "indexing",
];

interface IngestionStatusDotProps {
  status: IngestionStatusKey;
  size?: number;
  pulse?: boolean;
  className?: string;
}

export function IngestionStatusDot(props: IngestionStatusDotProps) {
  return (
    <StatusDot {...props} colorMap={INGESTION_STATUS_COLORS} pulseStatuses={ACTIVE_STATUSES} />
  );
}
