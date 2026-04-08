/**
 * Auto-scroll hook — scrolls to bottom when new messages or content blocks arrive.
 *
 * Cancel: user scrolls up (wheel/touchmove — only fires from real user input).
 * Resume: user scrolls down to the bottom (wheel/touchmove near bottom).
 *
 * Programmatic scrollIntoView never triggers cancel or resume — the scroll
 * event it generates is ignored via an isProgrammaticScroll guard.
 */
import { useEffect, useRef, useCallback, useState } from 'react';

export function useAutoScroll(messages: { length: number }, lastMsg: { isGenerating?: boolean; content?: any[] } | undefined) {
  const chatEndRef = useRef<HTMLDivElement>(null);
  const lastChatCountRef = useRef(0);
  const lastContentLenRef = useRef(0);
  const isNearBottomRef = useRef(true);
  const isProgrammaticScrollRef = useRef(false);
  const [showScrollButton, setShowScrollButton] = useState(false);

  const doScrollToBottom = useCallback(() => {
    isProgrammaticScrollRef.current = true;
    chatEndRef.current?.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
    // Clear the flag after the browser has processed the scroll.
    requestAnimationFrame(() => { isProgrammaticScrollRef.current = false; });
  }, []);

  const updateScrollButton = useCallback((container: Element) => {
    const { scrollTop, scrollHeight, clientHeight } = container;
    const distanceFromBottom = scrollHeight - scrollTop - clientHeight;
    const contentOverflows = scrollHeight > clientHeight * 1.5;
    setShowScrollButton(!isNearBottomRef.current && distanceFromBottom > 100 && contentOverflows);
  }, []);

  useEffect(() => {
    const endEl = chatEndRef.current;
    if (!endEl) return;
    const container = endEl.parentElement;
    if (!container) return;

    // wheel only fires from real user interaction.
    const onWheel = (e: WheelEvent) => {
      if (e.deltaY < 0) {
        // Scrolling up → cancel
        isNearBottomRef.current = false;
      } else if (e.deltaY > 0) {
        // Scrolling down → check if at bottom to resume
        const { scrollTop, scrollHeight, clientHeight } = container;
        const distanceFromBottom = scrollHeight - scrollTop - clientHeight;
        if (distanceFromBottom < 30) {
          isNearBottomRef.current = true;
        }
      }
      updateScrollButton(container);
    };

    let lastTouchY = 0;
    const onTouchStart = (e: TouchEvent) => {
      lastTouchY = e.touches[0].clientY;
    };
    const onTouchMove = (e: TouchEvent) => {
      const currentY = e.touches[0].clientY;
      if (currentY > lastTouchY) {
        // Finger dragging down = scrolling up → cancel
        isNearBottomRef.current = false;
      } else if (currentY < lastTouchY) {
        // Finger dragging up = scrolling down → check if at bottom
        const { scrollTop, scrollHeight, clientHeight } = container;
        const distanceFromBottom = scrollHeight - scrollTop - clientHeight;
        if (distanceFromBottom < 30) {
          isNearBottomRef.current = true;
        }
      }
      lastTouchY = currentY;
      updateScrollButton(container);
    };

    // Fallback: resume on scroll-to-bottom, but only from user scrolls.
    const onScroll = () => {
      if (isProgrammaticScrollRef.current) return;
      const { scrollTop, scrollHeight, clientHeight } = container;
      const distanceFromBottom = scrollHeight - scrollTop - clientHeight;
      if (distanceFromBottom <= 1) {
        isNearBottomRef.current = true;
      }
      updateScrollButton(container);
    };

    container.addEventListener('wheel', onWheel, { passive: true });
    container.addEventListener('touchstart', onTouchStart, { passive: true });
    container.addEventListener('touchmove', onTouchMove, { passive: true });
    container.addEventListener('scroll', onScroll, { passive: true });
    return () => {
      container.removeEventListener('wheel', onWheel);
      container.removeEventListener('touchstart', onTouchStart);
      container.removeEventListener('touchmove', onTouchMove);
      container.removeEventListener('scroll', onScroll);
    };
  }, [updateScrollButton]);

  const lastContentLen = lastMsg?.isGenerating ? (lastMsg.content?.length || 0) : 0;
  useEffect(() => {
    const newMessages = messages.length > lastChatCountRef.current;
    const newContentBlocks = lastContentLen > lastContentLenRef.current;
    if ((newMessages || newContentBlocks) && isNearBottomRef.current) {
      doScrollToBottom();
    }
    lastChatCountRef.current = messages.length;
    lastContentLenRef.current = lastContentLen;
  }, [messages.length, lastContentLen, doScrollToBottom]);

  const scrollToBottom = useCallback(() => {
    isNearBottomRef.current = true;
    setShowScrollButton(false);
    doScrollToBottom();
  }, [doScrollToBottom]);

  return { chatEndRef, scrollToBottom, showScrollButton };
}
