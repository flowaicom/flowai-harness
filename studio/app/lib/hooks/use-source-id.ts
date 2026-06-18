/**
 * Hook for role-aware source selection.
 *
 * Each Connect subsection has a "natural" database role (e.g. Discovery -> target,
 * Knowledge -> catalog). This hook resolves the natural default and lets the user
 * override it locally without mutating global store state.
 *
 * Source IDs in the new hierarchy model are workspace_database IDs which may
 * contain the role as a suffix (e.g. "default:target"). We match by checking
 * if the source's id ends with the role or if the source's name contains it.
 *
 * @module lib/hooks/use-source-id
 */

import { useEffect, useState } from "react";
import type { WorkspaceRole } from "~/lib/domain/workspace";
import { selectSources, useSourceCatalog } from "~/lib/stores/source-catalog";

/**
 * Resolve a source ID for a Connect subsection.
 *
 * - Initializes to the source whose `id` ends with the naturalRole
 * - Falls back to the first source, then `undefined`
 * - Re-initializes when the sources list changes (e.g. post-provisioning)
 */
export function useSourceId(naturalRole: WorkspaceRole): {
  sourceId: string | undefined;
  setSourceId: (id: string) => void;
} {
  const sources = useSourceCatalog(selectSources);

  const resolve = (): string | undefined => {
    // Try exact match first (legacy), then suffix match (new hierarchy: "ws:role")
    const match =
      sources.find((s) => s.id === naturalRole) ||
      sources.find((s) => s.id.endsWith(`:${naturalRole}`));
    if (match) return match.id;
    return sources.length > 0 ? sources[0].id : undefined;
  };

  const [sourceId, setSourceId] = useState<string | undefined>(resolve);

  // Re-initialize when sources list identity changes (handles provisioning reload)
  useEffect(() => {
    setSourceId(resolve());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resolve]);

  return { sourceId, setSourceId };
}
