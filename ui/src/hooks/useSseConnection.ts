/**
 * EventSource lifecycle: connection, seq dedup, reconnect with state resync.
 */
import { useEffect, useRef } from 'react';
import type { UiSseMessage } from '../types';
import { useUiStore } from '../stores/uiStore';
import { useChatStore } from '../stores/chatStore';
import { useAgentStore } from '../stores/agentStore';
import { useProjectStore } from '../stores/projectStore';


export interface SseConnectionOptions {
  onEvent: (item: UiSseMessage) => void;
  onParseError?: () => void;
  /** When provided, the SSE connection passes session_id to the server for
   *  server-side filtering. The connection reconnects when this value changes. */
  sessionId?: string | null;
  /** Called on reconnect (not initial connect) — use to resync state.
   *  When not provided, the default resyncState is used. */
  onReconnect?: () => void;
}

/** Refetch all critical state after an SSE reconnect to fill any gaps. */
function resyncState() {
  useChatStore.getState().fetchWorkspaceState();
  useAgentStore.getState().fetchAgentRuns();
  useProjectStore.getState().fetchSessions();
  useProjectStore.getState().fetchAllAgentTrees();
  useUiStore.getState().fetchPendingAskUser();
}

export function useSseConnection({ onEvent, onParseError, sessionId, onReconnect }: SseConnectionOptions) {
  const lastSeqRef = useRef(0);
  const hadConnectionRef = useRef(false);
  // Use refs so the EventSource doesn't need to be recreated when callbacks change
  const onEventRef = useRef(onEvent);
  const onParseErrorRef = useRef(onParseError);
  const onReconnectRef = useRef(onReconnect);

  useEffect(() => {
    onEventRef.current = onEvent;
    onParseErrorRef.current = onParseError;
    onReconnectRef.current = onReconnect;
  }, [onEvent, onParseError, onReconnect]);

  useEffect(() => {
    const url = sessionId
      ? `/api/events?session_id=${encodeURIComponent(sessionId)}`
      : '/api/events';
    const events = new EventSource(url);

    // Reset seq counter on (re)connect. This is safe because:
    // 1. Server restart resets seq to 0, so old seq values won't collide.
    // 2. EventSource auto-reconnects on disconnect, and we need to accept
    //    the server's new seq range without stale filtering.
    events.onopen = () => {
      const wasReconnecting = hadConnectionRef.current;
      lastSeqRef.current = 0;
      hadConnectionRef.current = true;
      useUiStore.getState().setSseStatus('connected');

      // On reconnect (not initial connect), resync state to fill any gaps
      if (wasReconnecting) {
        if (onReconnectRef.current) {
          onReconnectRef.current();
        } else {
          resyncState();
        }
      }
    };

    events.onerror = () => {
      // EventSource auto-reconnects, but we track the status for UI feedback
      if (hadConnectionRef.current) {
        useUiStore.getState().setSseStatus('reconnecting');
      }
    };

    events.onmessage = (e) => {
      try {
        const item = JSON.parse(e.data) as UiSseMessage;
        if (typeof item.seq === 'number') {
          if (item.seq <= lastSeqRef.current) return;
          lastSeqRef.current = item.seq;
        }
        onEventRef.current(item);
      } catch (err) {
        console.error('SSE parse error', err);
        onParseErrorRef.current?.();
      }
    };

    return () => {
      events.close();
      hadConnectionRef.current = false;
    };
  }, [sessionId]);
}
