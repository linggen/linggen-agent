/**
 * Room chat state — messages between users in a proxy room.
 * In-memory only, no persistence. Capped at 100 messages.
 */
import { create } from 'zustand';

export interface RoomChatMessage {
  id: string;
  senderId: string;
  senderName: string;
  avatarUrl: string | null;
  text: string;
  timestamp: number;
  isMine: boolean;
}

interface RoomChatState {
  messages: RoomChatMessage[];
  isOpen: boolean;
  unreadCount: number;
  addMessage: (msg: Omit<RoomChatMessage, 'id'>) => void;
  setOpen: (open: boolean) => void;
  clearUnread: () => void;
  clear: () => void;
}

const MAX_MESSAGES = 100;

let msgCounter = 0;

export const useRoomChatStore = create<RoomChatState>((set, get) => ({
  messages: [],
  isOpen: false,
  unreadCount: 0,

  addMessage: (msg) => {
    const id = `rc-${++msgCounter}`;
    set((s) => {
      const messages = [...s.messages, { ...msg, id }];
      if (messages.length > MAX_MESSAGES) messages.splice(0, messages.length - MAX_MESSAGES);
      return {
        messages,
        unreadCount: s.isOpen ? 0 : s.unreadCount + (msg.isMine ? 0 : 1),
      };
    });
  },

  setOpen: (open) => set({ isOpen: open, unreadCount: open ? 0 : get().unreadCount }),

  clearUnread: () => set({ unreadCount: 0 }),

  clear: () => set({ messages: [], unreadCount: 0 }),
}));
