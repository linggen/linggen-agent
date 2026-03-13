/** Options for mounting a ChatPanel */
export interface ChatPanelOptions {
  /** Base URL for Linggen server. Defaults to '' (same-origin). */
  serverUrl?: string;
  /** Skill name to bind the session to. */
  skillName?: string;
  /** Agent ID. Defaults to 'ling'. */
  agentId?: string;
  /** Existing session ID. If not provided, a new session is created. */
  sessionId?: string;
  /** Model ID to use. If not provided, uses server default. */
  modelId?: string;
  /** Chat panel title. */
  title?: string;
  /** Input placeholder text. */
  placeholder?: string;
  /** Additional CSS class for the container. */
  className?: string;
  /** Called when session is created or set. */
  onSessionCreated?: (sessionId: string) => void;
  /** Called when a complete message is received. */
  onMessage?: (message: ChatMessage) => void;
  /** Called for each streaming token. Receives the full accumulated text. */
  onStreamToken?: (fullText: string) => void;
  /** Called when streaming completes. Receives the final text. */
  onStreamEnd?: (text: string) => void;
}

/** A chat message */
export interface ChatMessage {
  id: string;
  role: 'user' | 'ai' | 'system';
  text: string;
  timestamp: number;
  isStreaming?: boolean;
}

/** Handle returned by mount() */
export interface ChatInstance {
  /** Send a message programmatically (calls API). */
  send: (text: string) => void;
  /** Add a display-only message without calling API. */
  addMessage: (role: ChatMessage['role'], text: string) => void;
  /** Clear all displayed messages. */
  clear: () => void;
  /** Destroy the chat panel and clean up. */
  destroy: () => void;
  /** Get the current session ID. */
  getSessionId: () => string | null;
  /** Update options dynamically. */
  setOptions: (opts: Partial<ChatPanelOptions>) => void;
}
