import { createRoot, Root } from 'react-dom/client';
import { createElement, createRef } from 'react';
import { ChatPanel } from '../components/ChatPanel';
import type { ChatPanelOptions, ChatInstance, ChatMessage } from '../types';

export function mount(
  selector: string | HTMLElement,
  options: ChatPanelOptions
): ChatInstance {
  const el = typeof selector === 'string'
    ? document.querySelector(selector)
    : selector;

  if (!el) throw new Error(`LinggenUI: element not found: ${selector}`);

  // Create React root
  const container = document.createElement('div');
  container.className = 'linggen-chat-container';
  el.appendChild(container);

  const root: Root = createRoot(container);
  const sendRef = createRef<((text: string) => void) | null>() as React.MutableRefObject<((text: string) => void) | null>;
  sendRef.current = null;
  const addMessageRef = createRef<((role: ChatMessage['role'], text: string) => void) | null>() as React.MutableRefObject<((role: ChatMessage['role'], text: string) => void) | null>;
  addMessageRef.current = null;
  const clearRef = createRef<(() => void) | null>() as React.MutableRefObject<(() => void) | null>;
  clearRef.current = null;

  let currentOptions = { ...options };
  let sessionId: string | null = options.sessionId || null;

  function render(opts: ChatPanelOptions) {
    root.render(
      createElement(ChatPanel, {
        ...opts,
        sendRef,
        addMessageRef,
        clearRef,
        onSessionCreated: (sid: string) => {
          sessionId = sid;
          opts.onSessionCreated?.(sid);
        },
      })
    );
  }

  render(currentOptions);

  const instance: ChatInstance = {
    send(text: string) {
      if (sendRef.current) {
        sendRef.current(text);
      }
    },

    addMessage(role: ChatMessage['role'], text: string) {
      if (addMessageRef.current) {
        addMessageRef.current(role, text);
      }
    },

    clear() {
      if (clearRef.current) {
        clearRef.current();
      }
    },

    destroy() {
      root.unmount();
      container.remove();
    },

    getSessionId() {
      return sessionId;
    },

    setOptions(opts: Partial<ChatPanelOptions>) {
      currentOptions = { ...currentOptions, ...opts };
      render(currentOptions);
    },
  };

  return instance;
}
