/**
 * Helpers for chatStore message mutations.
 *
 * Extracted from chatStore.ts to keep the store file focused on actions.
 */
import type { ChatMessage } from '../types';
import {
  stripEmbeddedStructuredJson,
  isPlanMessage,
  dedupPlanMessages,
} from '../lib/messageUtils';

/**
 * Subset of ChatMutationState used by mutation helpers.
 * Must stay in sync with the full ChatMutationState in chatStore.ts.
 * Cannot import from chatStore directly to avoid circular dependency.
 */
export interface ChatMutationState {
  messages: ChatMessage[];
  displayMessages: ChatMessage[];
  _messagesBySession: Record<string, ChatMessage[]>;
  _activeSessionId: string | null;
}

/** Compute display messages from raw messages. */
export function computeDisplay(messages: ChatMessage[]): ChatMessage[] {
  // Hide [BOARD_MOVE] messages — internal game state not meant for display
  const visible = messages.filter(m => !(m.role === 'user' && m.text?.startsWith('[BOARD_MOVE]')));
  const deduped = dedupPlanMessages(visible);
  const plans: ChatMessage[] = [];
  const nonPlans: ChatMessage[] = [];
  for (const msg of deduped) {
    if (isPlanMessage(msg)) {
      plans.push(msg);
    } else {
      if ((msg.from || msg.role) !== 'user' && !(msg.content && msg.content.length > 0)) {
        let text = msg.text || '';
        text = stripEmbeddedStructuredJson(text);
        if (msg.isGenerating) {
          text = text.replace(/\{\s*"type\b[^}]*$/s, '').trim();
        }
        if (text !== msg.text && text) {
          nonPlans.push({ ...msg, text });
          continue;
        }
      }
      nonPlans.push(msg);
    }
  }
  return [...nonPlans, ...plans];
}

/** Apply a mutation to the active session's messages and recompute displayMessages. */
export function mutate(fn: (msgs: ChatMessage[]) => ChatMessage[]): (s: ChatMutationState) => Partial<ChatMutationState> {
  return (s) => {
    const sid = s._activeSessionId || '__none__';
    const currentMsgs = s._messagesBySession[sid] || [];
    const messages = fn(currentMsgs);
    const displayMessages = computeDisplay(messages);
    return {
      messages,
      displayMessages,
      _messagesBySession: { ...s._messagesBySession, [sid]: messages },
    };
  };
}

/**
 * Fast-path mutation for token streaming: updates only the last message in
 * both `messages` and `displayMessages` without running `computeDisplay` over
 * the full array.  Falls back to `mutate` when the last message changes identity
 * (new message added, message removed, etc.).
 */
export function mutateLast(fn: (msgs: ChatMessage[]) => ChatMessage[]): (s: ChatMutationState) => Partial<ChatMutationState> {
  return (s) => {
    const sid = s._activeSessionId || '__none__';
    const currentMsgs = s._messagesBySession[sid] || [];
    const messages = fn(currentMsgs);

    // Fast path: if the array length is the same and only the last element changed,
    // patch displayMessages in-place instead of recomputing from scratch.
    if (
      messages.length === currentMsgs.length &&
      messages.length > 0 &&
      s.displayMessages.length > 0
    ) {
      const lastNew = messages[messages.length - 1];
      const lastDisplay = s.displayMessages[s.displayMessages.length - 1];
      // Match by identity: same role+from means we're updating the same bubble.
      // Skip fast path for plan messages — computeDisplay moves them to the end,
      // and the fast path would overwrite the plan entry with streaming token text.
      if (lastDisplay && lastNew.role === lastDisplay.role && lastNew.from === lastDisplay.from && !isPlanMessage(lastDisplay)) {
        const displayMessages = [...s.displayMessages];
        displayMessages[displayMessages.length - 1] = lastNew;
        return {
          messages,
          displayMessages,
          _messagesBySession: { ...s._messagesBySession, [sid]: messages },
        };
      }
    }

    // Fallback: full recompute (new message was added, etc.)
    const displayMessages = computeDisplay(messages);
    return {
      messages,
      displayMessages,
      _messagesBySession: { ...s._messagesBySession, [sid]: messages },
    };
  };
}
