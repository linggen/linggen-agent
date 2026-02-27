/**
 * SSE event router: builds a stable onEvent callback from app deps.
 */
import { useCallback, useEffect, useRef } from 'react';
import type { UiSseMessage } from '../types';
import { dispatchSseEvent, type SseHandlerDeps } from '../lib/sseEventHandlers';

export function useSseDispatch(deps: SseHandlerDeps): (item: UiSseMessage) => void {
  const depsRef = useRef(deps);
  useEffect(() => {
    depsRef.current = deps;
  }, [deps]);

  return useCallback((item: UiSseMessage) => {
    dispatchSseEvent(item, depsRef.current);
  }, []);
}
