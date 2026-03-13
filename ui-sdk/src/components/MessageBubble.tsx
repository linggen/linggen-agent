import type { ChatMessage } from '../types';
import { MarkdownContent } from './MarkdownContent';

export function MessageBubble({ message }: { message: ChatMessage }) {
  const { role, text, isStreaming } = message;

  return (
    <div className={`lc-msg lc-msg-${role}${isStreaming ? ' lc-streaming' : ''}`}>
      {isStreaming ? (
        <span className="lc-msg-text">{text}<span className="lc-cursor" /></span>
      ) : role === 'system' ? (
        <span className="lc-msg-text">{text}</span>
      ) : (
        <MarkdownContent text={text} />
      )}
    </div>
  );
}
