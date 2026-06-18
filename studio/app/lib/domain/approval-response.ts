export type ParsedApprovalPartial =
  | { readonly ok: true; readonly partial?: Record<string, unknown> }
  | { readonly ok: false; readonly error: string };

export function parseApprovalPartialJson(input: string): ParsedApprovalPartial {
  const trimmed = input.trim();
  if (!trimmed) return { ok: true };

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : "Partial JSON is invalid.",
    };
  }

  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return { ok: false, error: "Partial JSON must be a JSON object." };
  }

  return { ok: true, partial: parsed as Record<string, unknown> };
}
