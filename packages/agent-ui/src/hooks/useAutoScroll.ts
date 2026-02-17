import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

const BOTTOM_THRESHOLD = 40;

/**
 * Scroll-to-bottom behaviour: sticks to the bottom while streaming,
 * pauses when user scrolls up, resumes when user scrolls back down.
 */
export function useAutoScroll(
  containerRef: RefObject<HTMLElement | null>,
  deps: unknown[] = [],
) {
  const [isAtBottom, setIsAtBottom] = useState(true);
  const userScrolledUp = useRef(false);

  const checkBottom = useCallback(() => {
    const el = containerRef.current;
    if (!el) return true;
    return el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_THRESHOLD;
  }, [containerRef]);

  // Watch for scroll events
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const onScroll = () => {
      const atBottom = checkBottom();
      setIsAtBottom(atBottom);
      userScrolledUp.current = !atBottom;
    };

    el.addEventListener('scroll', onScroll, { passive: true });
    return () => el.removeEventListener('scroll', onScroll);
  }, [containerRef, checkBottom]);

  // Auto-scroll when deps change and we're at the bottom
  useEffect(() => {
    if (userScrolledUp.current) return;
    const el = containerRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  const scrollToBottom = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    userScrolledUp.current = false;
    el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
    setIsAtBottom(true);
  }, [containerRef]);

  return { isAtBottom, scrollToBottom };
}
