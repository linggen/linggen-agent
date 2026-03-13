/** Chat state — simple React state, no external deps */

import type { ChatMessage } from '../types';

export interface ChatState {
  messages: ChatMessage[];
  streamBuffer: string;
  isStreaming: boolean;
  isThinking: boolean;
  sessionId: string | null;
  modelId: string;
}

export function createInitialState(): ChatState {
  return {
    messages: [],
    streamBuffer: '',
    isStreaming: false,
    isThinking: false,
    sessionId: null,
    modelId: '',
  };
}

let nextId = 0;

export function createMessage(role: ChatMessage['role'], text: string): ChatMessage {
  return {
    id: `msg-${++nextId}-${Date.now()}`,
    role,
    text,
    timestamp: Date.now(),
  };
}
