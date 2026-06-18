/**
 * Ingestion status tag.
 *
 * Thin wrapper over shared StatusTag with ingestion-specific color + label maps.
 *
 * @module components/data/ingestion-status-tag
 */

import { StatusTag } from "~/components/shared/status-tag";
import { INGESTION_STATUS_COLORS, type IngestionStatusKey } from "~/lib/domain/data";

const LABELS: Record<IngestionStatusKey, string> = {
  queued: "Queued",
  discovering: "Discovering",
  profiling: "Profiling",
  enriching: "Enriching",
  extracting: "Extracting",
  indexing: "Indexing",
  completed: "Completed",
  failed: "Failed",
};

interface IngestionStatusTagProps {
  status: IngestionStatusKey;
  label?: string;
  className?: string;
}

export function IngestionStatusTag(props: IngestionStatusTagProps) {
  return <StatusTag {...props} colorMap={INGESTION_STATUS_COLORS} labelMap={LABELS} />;
}
