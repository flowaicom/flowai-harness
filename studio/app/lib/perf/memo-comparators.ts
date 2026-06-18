/**
 * Named memo comparators for React.memo (P27).
 *
 * Reusable comparator functions that prevent unnecessary re-renders
 * by comparing only the props that matter for each component type.
 *
 * Principles:
 * - Named functions for readability and debugging
 * - Type-safe comparators using generics
 * - Optimized for common UI patterns
 *
 * @module lib/perf/memo-comparators
 */

// ============================================================================
// Types
// ============================================================================

/**
 * Props with an ID field.
 */
interface WithId {
  id: string;
}

/**
 * Props with text content.
 */
interface WithText {
  text: string;
}

/**
 * Props with a state field.
 */
interface WithState<S extends string> {
  state: S;
}

/**
 * Props with streaming flag.
 */
interface WithStreaming {
  isStreaming?: boolean;
}

/**
 * Props with a parts array.
 */
interface WithParts<T> {
  parts: T[];
}

// ============================================================================
// Basic Comparators
// ============================================================================

/**
 * Compare by ID only.
 * Use for components where identity determines rendering.
 */
export function compareById<T extends WithId>(prev: T, next: T): boolean {
  return prev.id === next.id;
}

/**
 * Compare by text content only.
 * Use for simple text display components.
 */
export function compareByText<T extends WithText>(prev: T, next: T): boolean {
  return prev.text === next.text;
}

/**
 * Compare by state value only.
 * Use for state-driven components.
 */
export function compareByState<T extends WithState<string>, U extends WithState<string>>(
  prev: T,
  next: U
): boolean {
  return prev.state === next.state;
}

// ============================================================================
// Message Comparators
// ============================================================================

/**
 * Compare message props (P1, P59).
 *
 * For stable messages, compare by ID and parts reference.
 * For streaming messages, always re-render.
 */
export function compareMessageProps<T extends WithId & WithStreaming & WithParts<unknown>>(
  prev: T,
  next: T
): boolean {
  // Different IDs always differ
  if (prev.id !== next.id) return false;

  // Streaming state change
  if (prev.isStreaming !== next.isStreaming) return false;

  // Streaming messages always need to re-render
  if (next.isStreaming) return false;

  // Compare parts by reference (stable messages)
  return prev.parts === next.parts;
}

/**
 * Compare streaming message props (P45).
 *
 * Optimized for frequent updates during streaming:
 * - Compare length first (cheapest)
 * - Compare last part only (most common change)
 */
export function compareStreamingParts<T extends { type: string; text?: string }>(
  prev: WithParts<T>,
  next: WithParts<T>
): boolean {
  // Different lengths always differ
  if (prev.parts.length !== next.parts.length) return false;

  // Empty arrays are equal
  if (prev.parts.length === 0) return true;

  // Compare last part only (most likely to change)
  const prevLast = prev.parts[prev.parts.length - 1];
  const nextLast = next.parts[next.parts.length - 1];

  if (prevLast.type !== nextLast.type) return false;

  // For text parts, compare content
  if (prevLast.type === "text" && nextLast.type === "text") {
    return prevLast.text === nextLast.text;
  }

  // Default: reference equality
  return prevLast === nextLast;
}

// ============================================================================
// Tool Display Comparators
// ============================================================================

/**
 * Tool invocation props for comparison.
 */
interface ToolInvocationProps {
  toolCallId: string;
  toolName: string;
  state: string;
  result?: unknown;
}

/**
 * Compare tool invocation props (P44).
 *
 * Compares the fields that affect rendering.
 */
export function compareToolInvocation(
  prev: { part: ToolInvocationProps },
  next: { part: ToolInvocationProps }
): boolean {
  return (
    prev.part.toolCallId === next.part.toolCallId &&
    prev.part.state === next.part.state &&
    prev.part.toolName === next.part.toolName
  );
}

/**
 * Tool agent props for comparison.
 */
interface ToolAgentProps {
  toolCallId: string;
  agentName: string;
  state: string;
}

/**
 * Compare tool agent (sub-agent) props (P43).
 */
export function compareToolAgent(
  prev: { part: ToolAgentProps },
  next: { part: ToolAgentProps }
): boolean {
  return (
    prev.part.toolCallId === next.part.toolCallId &&
    prev.part.state === next.part.state &&
    prev.part.agentName === next.part.agentName
  );
}

// ============================================================================
// Latency Panel Comparators
// ============================================================================

/**
 * Metric card props for comparison.
 */
interface MetricCardProps {
  label: string;
  value: string | number | null | undefined;
}

/**
 * Compare metric card props (P60).
 */
export function compareMetricCard(prev: MetricCardProps, next: MetricCardProps): boolean {
  return prev.label === next.label && prev.value === next.value;
}

/**
 * Sub-agent row props for comparison.
 */
interface SubAgentRowProps {
  agentName: string;
  durationMs?: number;
  status?: string;
}

/**
 * Compare sub-agent row props (P61).
 */
export function compareSubAgentRow(prev: SubAgentRowProps, next: SubAgentRowProps): boolean {
  return (
    prev.agentName === next.agentName &&
    prev.durationMs === next.durationMs &&
    prev.status === next.status
  );
}

// ============================================================================
// Generic Comparators
// ============================================================================

/**
 * Create a comparator that checks specific props.
 *
 * @param keys - Props to compare
 * @returns A comparator function
 *
 * @example
 * ```tsx
 * const compareUser = createPropsComparator<UserProps>(['id', 'name', 'avatar']);
 * const MemoizedUser = memo(User, compareUser);
 * ```
 */
export function createPropsComparator<T extends Record<string, unknown>>(
  keys: (keyof T)[]
): (prev: T, next: T) => boolean {
  return (prev: T, next: T): boolean => {
    for (const key of keys) {
      if (!Object.is(prev[key], next[key])) {
        return false;
      }
    }
    return true;
  };
}

/**
 * Create a comparator that always re-renders for streaming items.
 *
 * @param baseComparator - Comparator to use when not streaming
 * @returns A comparator function
 */
export function withStreamingOverride<T extends WithStreaming>(
  baseComparator: (prev: T, next: T) => boolean
): (prev: T, next: T) => boolean {
  return (prev: T, next: T): boolean => {
    // Always re-render streaming items
    if (next.isStreaming) return false;

    // Streaming state changed
    if (prev.isStreaming !== next.isStreaming) return false;

    return baseComparator(prev, next);
  };
}

// ============================================================================
// Combined Comparators
// ============================================================================

/**
 * Compare by ID and state (common pattern).
 */
export function compareByIdAndState<T extends WithId & WithState<string>>(
  prev: T,
  next: T
): boolean {
  return prev.id === next.id && prev.state === next.state;
}

/**
 * Compare by ID, state, and a custom predicate.
 */
export function compareByIdStateAnd<T extends WithId & WithState<string>>(
  predicate: (prev: T, next: T) => boolean
): (prev: T, next: T) => boolean {
  return (prev: T, next: T): boolean => {
    if (prev.id !== next.id) return false;
    if (prev.state !== next.state) return false;
    return predicate(prev, next);
  };
}
