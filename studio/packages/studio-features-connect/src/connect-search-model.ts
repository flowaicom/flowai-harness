export type ConnectSearchMode =
  | "unified"
  | "semantic"
  | "fuzzyTable"
  | "fuzzyColumn"
  | "resolveTerm";

export const CONNECT_SEARCH_TABS: readonly { id: ConnectSearchMode; label: string }[] = [
  { id: "unified", label: "All" },
  { id: "semantic", label: "Semantic" },
  { id: "fuzzyTable", label: "Tables" },
  { id: "fuzzyColumn", label: "Columns" },
  { id: "resolveTerm", label: "Terms" },
];

const CONNECT_SEARCH_ITEM_TYPE_CATEGORY: Record<string, string> = {
  table: "discovery",
  schema: "discovery",
  column: "planning",
  enum: "planning",
  metric: "execution",
  knowledge: "knowledge",
  document: "knowledge",
  relationship: "discovery",
};

export function getConnectSearchItemCategory(itemType: string): string {
  return CONNECT_SEARCH_ITEM_TYPE_CATEGORY[itemType.toLowerCase()] ?? "knowledge";
}

export function buildConnectSearchAskPrompt(input: {
  readonly itemType: string;
  readonly name: string;
  readonly description?: string | null;
  readonly maxDescriptionLength?: number;
}): string {
  const maxDescriptionLength = input.maxDescriptionLength ?? 200;
  const description = input.description?.slice(0, maxDescriptionLength).trim();
  return `Tell me about ${input.itemType} "${input.name}". ${description ?? ""}`.trim();
}

export type ConnectSearchRequest =
  | {
      readonly kind: "catalog";
      readonly mode: "unified" | "semantic";
      readonly query: string;
    }
  | {
      readonly kind: "tool";
      readonly toolId: string;
      readonly input: Record<string, unknown>;
    };

export function resolveConnectSearchRequest(
  mode: ConnectSearchMode,
  query: string
): ConnectSearchRequest {
  if (mode === "unified" || mode === "semantic") {
    return {
      kind: "catalog",
      mode,
      query,
    };
  }

  return {
    kind: "tool",
    toolId: "search_catalog",
    input: {
      query,
      ...(mode === "fuzzyTable" ? { kinds: ["table"] } : {}),
      ...(mode === "fuzzyColumn" ? { kinds: ["column"] } : {}),
    },
  };
}

export interface ConnectToolSearchRow {
  readonly name: string;
  readonly itemType: string;
  readonly description: string;
  readonly score: number;
  readonly matchField?: string;
}

export function parseConnectToolSearchRows(data: string): ConnectToolSearchRow[] {
  try {
    const parsed = JSON.parse(data);

    if (Array.isArray(parsed.results)) {
      return parsed.results.map((item: Record<string, unknown>) => toConnectToolSearchRow(item));
    }

    const merged: ConnectToolSearchRow[] = [];
    for (const key of ["tables", "columns", "metrics"] as const) {
      const items = parsed[key];
      if (Array.isArray(items)) {
        for (const item of items) {
          merged.push(toConnectToolSearchRow(item));
        }
      }
    }
    if (merged.length > 0) {
      return merged;
    }

    if (Array.isArray(parsed)) {
      return parsed.map((item: Record<string, unknown>) => toConnectToolSearchRow(item));
    }

    return [];
  } catch {
    return [];
  }
}

function toConnectToolSearchRow(item: Record<string, unknown>): ConnectToolSearchRow {
  return {
    name: String(item.name ?? item.tableName ?? item.columnName ?? ""),
    itemType: String(item.itemType ?? item.item_type ?? item.kind ?? item.type ?? ""),
    description: String(item.description ?? item.content ?? ""),
    score: typeof item.score === "number" ? item.score : 0,
    matchField: item.matchField
      ? String(item.matchField)
      : item.match_field
        ? String(item.match_field)
        : undefined,
  };
}
