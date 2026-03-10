/**
 * SSE event router — simply returns the store-based dispatcher.
 */
import { useCallback } from 'react';
import type { UiSseMessage } from '../types';
import { dispatchSseEvent } from '../lib/sseEventHandlers';

export function useSseDispatch(): (item: UiSseMessage) => void {
  return useCallback((item: UiSseMessage) => {
    dispatchSseEvent(item);
  }, []);
}
