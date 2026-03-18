/**
 * SSE event router — simply returns the store-based dispatcher.
 * Optionally accepts a sessionId for per-widget session filtering.
 */
import { useCallback } from 'react';
import type { UiSseMessage } from '../types';
import { dispatchSseEvent } from '../lib/sseEventHandlers';

export function useSseDispatch(sessionId?: string | null): (item: UiSseMessage) => void {
  return useCallback((item: UiSseMessage) => {
    dispatchSseEvent(item, sessionId ?? undefined);
  }, [sessionId]);
}
