/**
 * Media query hooks for responsive behavior.
 *
 * SSR-compatible with fallback values to prevent hydration mismatches.
 * Uses addEventListener for modern browsers (no deprecated addListener).
 *
 * @module lib/hooks/use-media-query
 */

import { useEffect, useState } from "react";

// ============================================================================
// Core Hook
// ============================================================================

/**
 * Subscribe to a CSS media query.
 *
 * @param query - CSS media query string
 * @param fallback - Value to use on server and before hydration
 * @returns Whether the media query matches
 *
 * @example
 * ```tsx
 * const isDark = useMediaQuery("(prefers-color-scheme: dark)");
 * const isLarge = useMediaQuery("(min-width: 1024px)", false);
 * ```
 */
export function useMediaQuery(query: string, fallback = false): boolean {
  const [matches, setMatches] = useState(fallback);

  useEffect(() => {
    // SSR guard
    if (typeof window === "undefined") return;

    const mediaQuery = window.matchMedia(query);

    // Set initial value
    setMatches(mediaQuery.matches);

    // Define listener
    const listener = (event: MediaQueryListEvent) => {
      setMatches(event.matches);
    };

    // Add listener (modern API)
    mediaQuery.addEventListener("change", listener);

    // Cleanup
    return () => {
      mediaQuery.removeEventListener("change", listener);
    };
  }, [query]);

  return matches;
}

// ============================================================================
// Preset Breakpoint Hooks
// ============================================================================

/**
 * Check if viewport is mobile width (< 768px).
 * Fallback: false (assume desktop on server for layout calculations).
 */
export function useIsMobile(): boolean {
  return useMediaQuery("(max-width: 767px)", false);
}

/**
 * Check if viewport is tablet width (768px - 1023px).
 */
export function useIsTablet(): boolean {
  return useMediaQuery("(min-width: 768px) and (max-width: 1023px)", false);
}

/**
 * Check if viewport is desktop width (≥ 1024px).
 * Fallback: true (assume desktop on server).
 */
export function useIsDesktop(): boolean {
  return useMediaQuery("(min-width: 1024px)", true);
}

/**
 * Check if viewport is large desktop (≥ 1280px).
 */
export function useIsLargeDesktop(): boolean {
  return useMediaQuery("(min-width: 1280px)", false);
}

// ============================================================================
// Preference Hooks
// ============================================================================

/**
 * Check if user prefers dark color scheme.
 */
export function usePrefersDarkMode(): boolean {
  return useMediaQuery("(prefers-color-scheme: dark)", false);
}

/**
 * Check if user prefers light color scheme.
 */
export function usePrefersLightMode(): boolean {
  return useMediaQuery("(prefers-color-scheme: light)", true);
}

/**
 * Check if user prefers reduced motion.
 * Important for accessibility - disable animations when true.
 */
export function usePrefersReducedMotion(): boolean {
  return useMediaQuery("(prefers-reduced-motion: reduce)", false);
}

/**
 * Check if user prefers more contrast.
 */
export function usePrefersHighContrast(): boolean {
  return useMediaQuery("(prefers-contrast: more)", false);
}

// ============================================================================
// Device Capability Hooks
// ============================================================================

/**
 * Check if device supports hover (has a pointing device).
 * False on touch-only devices.
 */
export function useCanHover(): boolean {
  return useMediaQuery("(hover: hover)", true);
}

/**
 * Check if device has a coarse pointer (touch).
 */
export function useHasCoarsePointer(): boolean {
  return useMediaQuery("(pointer: coarse)", false);
}

/**
 * Check if device has a fine pointer (mouse/trackpad).
 */
export function useHasFinePointer(): boolean {
  return useMediaQuery("(pointer: fine)", true);
}
