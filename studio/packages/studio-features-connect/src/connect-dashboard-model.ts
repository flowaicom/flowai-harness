export interface ConnectExplorePromptInput {
  readonly tableCount: number;
  readonly tableNames: readonly string[];
  readonly knowledgeCount: number;
  readonly documentCount: number;
}

export function summarizeConnectAvailableContext(
  input: ConnectExplorePromptInput,
  maxTableNames = 8
): string[] {
  const summary: string[] = [];
  if (input.tableCount > 0) {
    const names = input.tableNames.slice(0, maxTableNames).join(", ");
    const suffix =
      input.tableNames.length > maxTableNames
        ? ` and ${input.tableNames.length - maxTableNames} more`
        : "";
    if (names.length > 0) {
      summary.push(
        `${input.tableCount} table${input.tableCount > 1 ? "s" : ""} (${names}${suffix})`
      );
    } else {
      summary.push(`${input.tableCount} table${input.tableCount > 1 ? "s" : ""}`);
    }
  }
  if (input.knowledgeCount > 0) {
    summary.push(`${input.knowledgeCount} knowledge item${input.knowledgeCount > 1 ? "s" : ""}`);
  }
  if (input.documentCount > 0) {
    summary.push(`${input.documentCount} document${input.documentCount > 1 ? "s" : ""}`);
  }
  return summary;
}

export function buildConnectExplorePrompt(input: ConnectExplorePromptInput): string {
  const parts = summarizeConnectAvailableContext(input);
  const context = parts.length > 0 ? ` I have ${parts.join(", ")} available.` : "";
  return `I'd like to explore and analyze the data.${context} What can you help me with?`;
}
