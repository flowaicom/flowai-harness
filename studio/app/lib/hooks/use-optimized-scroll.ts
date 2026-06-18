/**
 * Optimized scroll hook with manual scroll detection.
 *
 * Features:
 * - RAF-throttled scrolling for smooth animations
 * - Manual scroll detection (stops auto-scroll when user scrolls up)
 * - Instant vs smooth behavior based on streaming state
 *
 * @module lib/hooks/use-optimized-scroll
 */

import { useCallback, useEffect, useRef } from "react";
import { cancelRafThrottle, rafThrottle } from "~/lib/perf/scheduler";

interface UseOptimizedScrollOptions {
  /** Threshold in pixels to consider "at bottom" */
  bottomThreshold?: number;
  /** ID for RAF deduplication */
  scrollId?: string;
}

interface UseOptimizedScrollReturn {
  /** Scroll to bottom with smooth behavior */
  scrollToBottom: () => void;
  /** Scroll to bottom instantly (for streaming) */
  scrollToBottomInstant: () => void;
  /** Mark that user has manually scrolled */
  markManualScroll: () => void;
  /** Reset manual scroll flag */
  resetManualScroll: () => void;
  /** Check if user has manually scrolled */
  isManuallyScrolled: () => boolean;
  /** Check if currently at bottom */
  isAtBottom: () => boolean;
}

export function useOptimizedScroll(
  targetRef: React.RefObject<HTMLElement | null>,
  options: UseOptimizedScrollOptions = {}
): UseOptimizedScrollReturn {
  const { bottomThreshold = 100, scrollId = "scroll-to-bottom" } = options;

  const hasManuallyScrolledRef = useRef(false);
  const lastScrollTopRef = useRef(0);

  // Check if scroll container is at bottom
  const isAtBottom = useCallback((): boolean => {
    const element = targetRef.current;
    if (!element) return true;

    const { scrollTop, scrollHeight, clientHeight } = element;
    return scrollHeight - scrollTop - clientHeight < bottomThreshold;
  }, [targetRef, bottomThreshold]);

  // Scroll to bottom with smooth behavior
  const scrollToBottom = useCallback(() => {
    if (hasManuallyScrolledRef.current) return;

    rafThrottle(scrollId, () => {
      targetRef.current?.scrollTo({
        top: targetRef.current.scrollHeight,
        behavior: "smooth",
      });
    });
  }, [targetRef, scrollId]);

  // Scroll to bottom instantly (for streaming - no animation lag)
  const scrollToBottomInstant = useCallback(() => {
    if (hasManuallyScrolledRef.current) return;

    rafThrottle(`${scrollId}-instant`, () => {
      const element = targetRef.current;
      if (element) {
        element.scrollTop = element.scrollHeight;
      }
    });
  }, [targetRef, scrollId]);

  // Mark manual scroll
  const markManualScroll = useCallback(() => {
    hasManuallyScrolledRef.current = true;
  }, []);

  // Reset manual scroll flag
  const resetManualScroll = useCallback(() => {
    hasManuallyScrolledRef.current = false;
  }, []);

  // Check if manually scrolled
  const isManuallyScrolled = useCallback(() => {
    return hasManuallyScrolledRef.current;
  }, []);

  // Detect manual scroll (user scrolling up)
  useEffect(() => {
    const element = targetRef.current;
    if (!element) return;

    const handleScroll = () => {
      const currentScrollTop = element.scrollTop;
      const scrollingUp = currentScrollTop < lastScrollTopRef.current;
      lastScrollTopRef.current = currentScrollTop;

      // User scrolled up significantly - mark as manual scroll
      if (scrollingUp && !isAtBottom()) {
        hasManuallyScrolledRef.current = true;
      }

      // User scrolled back to bottom - reset manual scroll
      if (isAtBottom()) {
        hasManuallyScrolledRef.current = false;
      }
    };

    element.addEventListener("scroll", handleScroll, { passive: true });

    return () => {
      element.removeEventListener("scroll", handleScroll);
      cancelRafThrottle(scrollId);
      cancelRafThrottle(`${scrollId}-instant`);
    };
  }, [targetRef, scrollId, isAtBottom]);

  return {
    scrollToBottom,
    scrollToBottomInstant,
    markManualScroll,
    resetManualScroll,
    isManuallyScrolled,
    isAtBottom,
  };
}
