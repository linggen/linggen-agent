// Linggen UI SDK — Chat component library
import './styles/chat.css';

// React components (for direct import)
export { ChatPanel } from './components/ChatPanel';
export { MessageBubble } from './components/MessageBubble';
export { MarkdownContent } from './components/MarkdownContent';
export { ThinkingIndicator } from './components/ThinkingIndicator';
export { ChatInput } from './components/ChatInput';

// Types
export type { ChatPanelOptions, ChatMessage, ChatInstance } from './types';

// Mount API (for vanilla JS / <script> tag usage)
export { mount } from './lib/mount';
