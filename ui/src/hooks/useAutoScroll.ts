/**
 * Auto-scroll hook — scrolls to bottom when new messages or content blocks arrive.
 */
import { useEffect, useRef, useCallback } from 'react';

export function useAutoScroll(messages: { length: number }, lastMsg: { isGenerating?: boolean; content?: any[] } | undefined) {
  const chatEndRef = useRef<HTMLDivElement>(null);
  const lastChatCountRef = useRef(0);
  const lastContentLenRef = useRef(0);
  const isNearBottomRef = useRef(true);
  const chatScrollContainerRef = useRef<HTMLElement | null>(null);

  const lastScrollTopRef = useRef(0);

  useEffect(() => {
    const endEl = chatEndRef.current;
    if (!endEl) return;
    const container = endEl.parentElement;
    if (!container) return;
    chatScrollContainerRef.current = container;
    lastScrollTopRef.current = container.scrollTop;
    const onScroll = () => {
      const { scrollTop, scrollHeight, clientHeight } = container;
      const distanceFromBottom = scrollHeight - scrollTop - clientHeight;
      // User scrolled up → stop auto-scroll
      if (scrollTop < lastScrollTopRef.current) {
        isNearBottomRef.current = false;
      }
      // User scrolled back to bottom → resume auto-scroll
      if (distanceFromBottom <= 1) {
        isNearBottomRef.current = true;
      }
      lastScrollTopRef.current = scrollTop;
    };
    container.addEventListener('scroll', onScroll, { passive: true });
    return () => container.removeEventListener('scroll', onScroll);
  }, []);

  const lastContentLen = lastMsg?.isGenerating ? (lastMsg.content?.length || 0) : 0;
  useEffect(() => {
    const newMessages = messages.length > lastChatCountRef.current;
    const newContentBlocks = lastContentLen > lastContentLenRef.current;
    if ((newMessages || newContentBlocks) && isNearBottomRef.current) {
      chatEndRef.current?.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
    }
    lastChatCountRef.current = messages.length;
    lastContentLenRef.current = lastContentLen;
  }, [messages.length, lastContentLen]);

  const scrollToBottom = useCallback(() => {
    isNearBottomRef.current = true;
    chatEndRef.current?.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
  }, []);

  return { chatEndRef, scrollToBottom };
}
