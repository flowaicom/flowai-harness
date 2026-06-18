export type ConnectDiscoveryTableLoadResult<TTable = unknown> =
  | { readonly kind: "idle" }
  | { readonly kind: "success"; readonly tables: readonly TTable[] }
  | { readonly kind: "error"; readonly message: string }
  | { readonly kind: "aborted" };

interface ConnectDiscoveryListSuccess<TTable> {
  readonly _tag: "Ok";
  readonly value: readonly TTable[];
}

interface ConnectDiscoveryListFailure {
  readonly _tag: "Err";
  readonly error: {
    readonly message?: string | null;
  };
}

type ConnectDiscoveryListResult<TTable> =
  | ConnectDiscoveryListSuccess<TTable>
  | ConnectDiscoveryListFailure;

export async function loadConnectDiscoveryTables<TTable>(params: {
  readonly hasTarget: boolean;
  readonly signal?: AbortSignal;
  readonly loadTables: (options: {
    readonly signal?: AbortSignal;
  }) => Promise<ConnectDiscoveryListResult<TTable>>;
  readonly errorMessage?: string;
}): Promise<ConnectDiscoveryTableLoadResult<TTable>> {
  if (!params.hasTarget) {
    return { kind: "idle" };
  }

  if (params.signal?.aborted) {
    return { kind: "aborted" };
  }

  const result = await params.loadTables({ signal: params.signal });

  if (params.signal?.aborted) {
    return { kind: "aborted" };
  }

  if (result._tag !== "Ok") {
    return {
      kind: "error",
      message: result.error.message || params.errorMessage || "Failed to load tables",
    };
  }

  return {
    kind: "success",
    tables: result.value,
  };
}

export interface ConnectTableExplorePromptInput {
  readonly schemaName: string;
  readonly tableName: string;
  readonly columnNames: readonly string[];
  readonly totalColumnCount: number;
}

export function summarizeConnectTableColumns(
  input: ConnectTableExplorePromptInput,
  maxColumnNames = 12
): string | null {
  if (input.totalColumnCount <= 0 || input.columnNames.length <= 0) {
    return null;
  }

  const names = input.columnNames.slice(0, maxColumnNames).join(", ");
  const suffix =
    input.totalColumnCount > maxColumnNames
      ? ` and ${input.totalColumnCount - maxColumnNames} more`
      : "";

  return `${names}${suffix}`;
}

export function buildConnectTableExplorePrompt(
  input: ConnectTableExplorePromptInput,
  maxColumnNames = 12
): string {
  const qualifiedTableName = `${input.schemaName}.${input.tableName}`;
  const summarizedColumns = summarizeConnectTableColumns(input, maxColumnNames);

  if (summarizedColumns) {
    return `Describe the table ${qualifiedTableName} (columns: ${summarizedColumns}). What kinds of queries or analyses can I run on it?`;
  }

  return `Describe the table ${qualifiedTableName}. What kinds of queries or analyses can I run on it?`;
}
