/**
 * Shallow comparison utilities for Zustand selectors.
 *
 * Use Zustand's official useShallow for selector memoization.
 * This module re-exports Zustand's built-in and provides utility functions.
 *
 * @module lib/hooks/use-shallow
 */

// Re-export Zustand's official useShallow (handles memoization correctly)
export { useShallow } from "zustand/react/shallow";

// ============================================================================
// Shallow Equal Implementation (utility function)
// ============================================================================

/**
 * Shallow compare two values.
 *
 * For objects: compares keys and values (one level deep)
 * For arrays: compares length and each element
 * For primitives: strict equality
 *
 * @param a - First value
 * @param b - Second value
 * @returns True if values are shallowly equal
 */
export function shallowEqual<T>(a: T, b: T): boolean {
  // Strict equality (handles primitives and same reference)
  if (Object.is(a, b)) {
    return true;
  }

  // Different types or one is null/undefined
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) {
    return false;
  }

  // Arrays
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) {
      return false;
    }
    for (let i = 0; i < a.length; i++) {
      if (!Object.is(a[i], b[i])) {
        return false;
      }
    }
    return true;
  }

  // One is array, other is not
  if (Array.isArray(a) !== Array.isArray(b)) {
    return false;
  }

  // Objects
  const keysA = Object.keys(a as Record<string, unknown>);
  const keysB = Object.keys(b as Record<string, unknown>);

  if (keysA.length !== keysB.length) {
    return false;
  }

  for (const key of keysA) {
    if (
      !Object.hasOwn(b, key) ||
      !Object.is((a as Record<string, unknown>)[key], (b as Record<string, unknown>)[key])
    ) {
      return false;
    }
  }

  return true;
}

// ============================================================================
// Memoized Selector Factories (create once, reuse)
// ============================================================================

/**
 * Create a memoized selector factory for picking specific keys from state.
 *
 * Unlike the original createShallowPick, this returns a stable selector
 * function that can be defined once at module level and reused.
 *
 * @param keys - Array of keys to pick from state
 * @returns A selector function that picks those keys
 *
 * @example
 * ```tsx
 * // Define once at module level
 * const selectUserInfo = createPickSelector<UserState>(['name', 'email']);
 *
 * // Use in component with useShallow
 * const { name, email } = useStore(useShallow(selectUserInfo));
 * ```
 */
export function createPickSelector<S extends Record<string, unknown>>(
  keys: readonly (keyof S)[]
): (state: S) => Pick<S, (typeof keys)[number]> {
  return (state: S): Pick<S, (typeof keys)[number]> => {
    const result = {} as Pick<S, (typeof keys)[number]>;
    for (const key of keys) {
      result[key] = state[key];
    }
    return result;
  };
}

// Legacy export for backwards compatibility (deprecated)
/** @deprecated Use createPickSelector with useShallow instead */
export const createShallowPick = createPickSelector;
