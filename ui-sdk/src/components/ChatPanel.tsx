import { useCallback, useEffect, useRef, useState } from 'react';
import type { ChatMessage, ChatPanelOptions } from '../types';
import { createSession, sendChat, fetchDefaultModel } from '../api/client';
import { connectSSE } from '../api/sse';
import { createMessage } from '../state/chat-store';
import { MessageBubble } from './MessageBubble';
import { ThinkingIndicator } from './ThinkingIndicator';
import { ChatInput } from './ChatInput';

export interface ChatPanelProps extends ChatPanelOptions {
  /** Imperatively send a message (set by parent via ref or callback). */
  sendRef?: React.MutableRefObject<((text: string) => void) | null>;
  /** Imperatively add a display-only message (set by parent via ref or callback). */
  addMessageRef?: React.MutableRefObject<((role: ChatMessage['role'], text: string) => void) | null>;
  /** Imperatively clear all messages. */
  clearRef?: React.MutableRefObject<(() => void) | null>;
}

export function ChatPanel(props: ChatPanelProps) {
  const {
    serverUrl = '',
    skillName,
    agentId = 'ling',
    sessionId: initialSessionId,
    modelId: initialModelId,
    title,
    placeholder,
    className,
    onSessionCreated,
    onMessage,
    onStreamToken,
    onStreamEnd,
    sendRef,
    addMessageRef,
    clearRef,
  } = props;

  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [isThinking, setIsThinking] = useState(false);
  const [sessionId, setSessionId] = useState<string | null>(initialSessionId || null);
  const [modelId, setModelId] = useState(initialModelId || '');

  const streamBufferRef = useRef('');
  const streamMsgIdRef = useRef<string | null>(null);
  const isBoardMoveRef = useRef(false);
  const eventSourceRef = useRef<EventSource | null>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const initRef = useRef(false);
  const onMessageRef = useRef(onMessage);
  const onStreamEndRef = useRef(onStreamEnd);
  const onStreamTokenRef = useRef(onStreamToken);

  // Keep callback refs current
  useEffect(() => { onMessageRef.current = onMessage; }, [onMessage]);
  useEffect(() => { onStreamEndRef.current = onStreamEnd; }, [onStreamEnd]);
  useEffect(() => { onStreamTokenRef.current = onStreamToken; }, [onStreamToken]);

  // Add a message to display (no API call)
  const addDisplayMessage = useCallback((role: ChatMessage['role'], text: string) => {
    const msg = createMessage(role, text);
    setMessages(prev => [...prev, msg]);
  }, []);

  // Add a system message
  const addSystemMessage = useCallback((text: string) => {
    addDisplayMessage('system', text);
  }, [addDisplayMessage]);

  // Auto-scroll to bottom
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ block: 'end' });
  }, [messages, isThinking]);

  // Finalize a completed stream
  const finalizeStream = useCallback(() => {
    const text = streamBufferRef.current;
    const msgId = streamMsgIdRef.current;

    // Always clear refs and thinking state, even on duplicate calls
    streamBufferRef.current = '';
    streamMsgIdRef.current = null;
    setIsThinking(false);

    // Always clear streaming flag on any remaining messages
    setMessages(prev =>
      prev.map(m => m.isStreaming ? { ...m, isStreaming: false } : m)
    );

    if (!text) return;

    onStreamEndRef.current?.(text);

    if (isBoardMoveRef.current) {
      // Board move — notify parent, don't show in chat
      const msg = createMessage('ai', text);
      onMessageRef.current?.(msg);
      isBoardMoveRef.current = false;
    } else if (msgId) {
      // Regular chat — finalize streaming message with final text
      setMessages(prev =>
        prev.map(m =>
          m.id === msgId ? { ...m, text, isStreaming: false } : m
        )
      );
      const msg = createMessage('ai', text);
      msg.id = msgId;
      onMessageRef.current?.(msg);
    }
  }, []);

  // SSE event handler
  const handleSSE = useCallback((data: Record<string, unknown>) => {
    const kind = data.kind as string;

    if (kind === 'token' && !(data.data as Record<string, unknown>)?.thinking) {
      setIsThinking(false);
      const tokenText = (data.text as string) || '';
      streamBufferRef.current += tokenText;

      onStreamTokenRef.current?.(streamBufferRef.current);

      // During board moves, don't show streaming in chat
      if (!isBoardMoveRef.current) {
        const buffer = streamBufferRef.current;
        setMessages(prev => {
          const existing = streamMsgIdRef.current;
          if (existing) {
            return prev.map(m =>
              m.id === existing ? { ...m, text: buffer } : m
            );
          } else {
            const msg = createMessage('ai', buffer);
            msg.isStreaming = true;
            streamMsgIdRef.current = msg.id;
            return [...prev, msg];
          }
        });
      }

      // Check for stream done
      if ((data.phase as string) === 'done') {
        finalizeStream();
      }
    }

    if (kind === 'turn_complete') {
      finalizeStream();
    }
  }, [finalizeStream]);

  // Initialize: resolve model, create session, connect SSE
  useEffect(() => {
    if (initRef.current) return;
    initRef.current = true;

    (async () => {
      // Resolve model
      let model = modelId;
      if (!model) {
        const def = await fetchDefaultModel(serverUrl);
        if (def) {
          model = def;
          setModelId(def);
        }
      }

      // Create or reuse session
      let sid = sessionId;
      if (!sid) {
        try {
          const sess = await createSession(serverUrl, `chat-${Date.now()}`, skillName);
          sid = sess.id;
          setSessionId(sid);
          onSessionCreated?.(sid);
        } catch (err) {
          addSystemMessage(`Failed to create session: ${(err as Error).message}`);
          return;
        }
      }

      // Connect SSE
      if (sid) {
        eventSourceRef.current = connectSSE(serverUrl, sid, handleSSE);
      }
    })();

    return () => {
      eventSourceRef.current?.close();
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Expose send function
  const sendMessage = useCallback(async (text: string) => {
    if (!sessionId || !modelId) return;

    // Check if this is a board move (hidden from chat)
    const isBoardMove = text.startsWith('[BOARD_MOVE]');
    isBoardMoveRef.current = isBoardMove;

    if (!isBoardMove) {
      const msg = createMessage('user', text);
      setMessages(prev => [...prev, msg]);
    }

    setIsThinking(true);

    try {
      await sendChat(serverUrl, {
        skillName,
        agentId,
        sessionId,
        modelId,
        message: text,
      });
    } catch (err) {
      setIsThinking(false);
      addSystemMessage(`Error: ${(err as Error).message}`);
    }
  }, [sessionId, modelId, serverUrl, skillName, agentId, addSystemMessage]);

  // Expose refs
  useEffect(() => {
    if (sendRef) sendRef.current = sendMessage;
  }, [sendMessage, sendRef]);

  useEffect(() => {
    if (addMessageRef) addMessageRef.current = addDisplayMessage;
  }, [addDisplayMessage, addMessageRef]);

  useEffect(() => {
    if (clearRef) clearRef.current = () => setMessages([]);
  }, [clearRef]);

  return (
    <div className={`lc-root${className ? ' ' + className : ''}`}>
      {title && <div className="lc-header">{title}</div>}
      <div className="lc-messages">
        {messages.map(msg => (
          <MessageBubble key={msg.id} message={msg} />
        ))}
        {isThinking && <ThinkingIndicator />}
        <div ref={messagesEndRef} />
      </div>
      <ChatInput
        placeholder={placeholder}
        onSend={sendMessage}
      />
    </div>
  );
}

ChatPanel.displayName = 'LinggenChatPanel';
