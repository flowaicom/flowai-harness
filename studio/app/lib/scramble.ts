/**
 * PII Scrambling Layer — Demo Mode
 *
 * Pure presentation-layer scrambling for showing the product to external
 * audiences without exposing business data. Scrambles alphabetic characters
 * via a fixed derangement (no letter maps to itself), preserving:
 * - Digits (0-9)
 * - Punctuation and symbols
 * - Whitespace and line breaks
 * - Character case (upper/lower)
 * - Text length and layout
 *
 * The scrambling is deterministic: same input always produces same output,
 * so the UI looks consistent across renders.
 *
 * @module lib/scramble
 */

import { useCallback, useEffect, useRef } from "react";
import { useAgentConfig } from "~/lib/stores/settings-store";

// ============================================================================
// Permutation Table (derangement — no fixed points)
// ============================================================================

// Maps each letter to a different letter. Verified: no letter maps to itself.
const LOWER_FROM = "abcdefghijklmnopqrstuvwxyz";
const LOWER_TO = "qwertyuiopasdfghjklzxcvbnm";

const forwardMap = new Map<string, string>();
for (let i = 0; i < 26; i++) {
  forwardMap.set(LOWER_FROM[i], LOWER_TO[i]);
  forwardMap.set(LOWER_FROM[i].toUpperCase(), LOWER_TO[i].toUpperCase());
}

// ============================================================================
// Core Functions
// ============================================================================

/**
 * Scramble text character-by-character.
 * Letters are permuted, everything else passes through unchanged.
 */
export function scrambleText(text: string): string {
  const out = new Array<string>(text.length);
  for (let i = 0; i < text.length; i++) {
    out[i] = forwardMap.get(text[i]) ?? text[i];
  }
  return out.join("");
}

/**
 * Identity function — returned when scrambling is disabled.
 */
const identity = <T>(x: T): T => x;

// ============================================================================
// React Hook
// ============================================================================

/**
 * Hook providing a scramble function gated by the piiScramble feature flag.
 *
 * When disabled, returns identity (zero overhead at call sites).
 * When enabled, returns scrambleText for all text content.
 *
 * The returned `s` reference is stable across renders when `enabled`
 * is unchanged (it's a module-level function, not a closure).
 *
 * Usage:
 * ```tsx
 * const { s } = useScramble();
 * return <span>{s(productName)}</span>;
 * ```
 */
export function useScramble(): {
  enabled: boolean;
  /** Scramble plain text (preserves digits, punctuation, whitespace) */
  s: (text: string) => string;
} {
  const enabled = useAgentConfig((state) => state.featureFlags.piiScramble);
  return {
    enabled,
    s: enabled ? scrambleText : identity,
  };
}

// ============================================================================
// ScrambleZone — MutationObserver-based DOM text node scrambling
// ============================================================================

/**
 * Walks all text nodes under `root` and scrambles their content.
 * Stores originals in the provided Map for later restoration.
 */
function scrambleTextNodes(root: Node, originals: Map<Text, string>) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const textNode = node as Text;
    if (!originals.has(textNode)) {
      originals.set(textNode, textNode.data);
    }
    // Scramble using the stored original to avoid double-scrambling
    const original = originals.get(textNode) ?? textNode.data;
    const scrambled = scrambleText(original);
    if (textNode.data !== scrambled) {
      textNode.data = scrambled;
    }
  }
}

/**
 * Restore original text from the saved map.
 */
function unscrambleTextNodes(originals: Map<Text, string>) {
  for (const [node, original] of originals) {
    if (node.parentNode) {
      node.data = original;
    }
  }
}

/**
 * Hook for scrambling text nodes inside a container ref.
 * Intended for external components where we can't use the
 * React-level `useScramble()` hook.
 *
 * Attaches a MutationObserver to handle dynamic content updates.
 *
 * Usage:
 * ```tsx
 * function ScrambleZone({ children }: { children: ReactNode }) {
 *   const ref = useScrambleZone();
 *   return <div ref={ref}>{children}</div>;
 * }
 * ```
 */
export function useScrambleZone(): React.RefCallback<HTMLElement> {
  const enabled = useAgentConfig((state) => state.featureFlags.piiScramble);
  const originals = useRef(new Map<Text, string>());
  const observerRef = useRef<MutationObserver | null>(null);
  const elementRef = useRef<HTMLElement | null>(null);

  // Single effect for setup/teardown — avoids split-cleanup ordering bug
  // where originals would be cleared before unscramble could read them.
  useEffect(() => {
    const el = elementRef.current;
    if (!el) return;

    if (enabled) {
      scrambleTextNodes(el, originals.current);

      // Observe for dynamic content
      // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: inherent mutation type × node type branching
      const observer = new MutationObserver((mutations) => {
        for (const m of mutations) {
          if (m.type === "childList") {
            for (const node of m.addedNodes) {
              if (node instanceof HTMLElement) {
                scrambleTextNodes(node, originals.current);
              } else if (node instanceof Text) {
                if (!originals.current.has(node)) {
                  originals.current.set(node, node.data);
                }
                const orig = originals.current.get(node) ?? node.data;
                node.data = scrambleText(orig);
              }
            }
          } else if (m.type === "characterData" && m.target instanceof Text) {
            const textNode = m.target;
            // If the new data differs from our scrambled version, it's a React update
            const storedOriginal = originals.current.get(textNode);
            if (
              storedOriginal !== textNode.data &&
              scrambleText(storedOriginal ?? "") !== textNode.data
            ) {
              originals.current.set(textNode, textNode.data);
              textNode.data = scrambleText(textNode.data);
            }
          }
        }
      });

      observer.observe(el, { childList: true, subtree: true, characterData: true });
      observerRef.current = observer;
    } else {
      observerRef.current?.disconnect();
      observerRef.current = null;
      unscrambleTextNodes(originals.current);
      originals.current.clear();
    }

    return () => {
      observerRef.current?.disconnect();
      observerRef.current = null;
      // On unmount or before re-run: restore text so DOM is clean
      unscrambleTextNodes(originals.current);
      originals.current.clear();
    };
  }, [enabled]);

  // Ref callback — called when the element mounts/unmounts
  return useCallback(
    (el: HTMLElement | null) => {
      elementRef.current = el;
      if (el && enabled) {
        scrambleTextNodes(el, originals.current);
      }
    },
    [enabled]
  );
}
