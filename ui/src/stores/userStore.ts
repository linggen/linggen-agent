/**
 * User identity, permission, and connection state.
 *
 * Two independent axes:
 * - userType: "owner" or "consumer" — set at connection time, never changes.
 * - userPermission: "admin"|"edit"|"read"|"chat" — what the agent can do.
 */
import { create } from 'zustand';

interface UserState {
  /** "owner" or "consumer" — set at connection time. */
  userType: 'owner' | 'consumer';
  userId: string | null;
  userPermission: 'admin' | 'edit' | 'read' | 'chat' | 'pending';
  userRoomName: string | null;
  userTokenBudget: number | null;
  connectionStatus: 'connected' | 'reconnecting' | 'disconnected';

  setUserType: (userType: 'owner' | 'consumer') => void;
  setUserId: (userId: string) => void;
  setUserInfo: (permission: string, roomName?: string | null, tokenBudget?: number | null) => void;
  setConnectionStatus: (status: 'connected' | 'reconnecting' | 'disconnected') => void;
}

const isRemote = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

export const useUserStore = create<UserState>((set) => ({
  userType: isRemote ? 'consumer' : 'owner',
  userId: null,
  userPermission: isRemote ? 'pending' as any : 'admin',
  userRoomName: null,
  userTokenBudget: null,
  connectionStatus: isRemote ? 'disconnected' : 'connected',

  setUserType: (userType) => set({ userType }),
  setUserId: (userId) => set({ userId }),
  setUserInfo: (permission, roomName, tokenBudget) => set({
    userPermission: permission as any,
    userRoomName: roomName ?? null,
    userTokenBudget: tokenBudget ?? null,
  }),
  setConnectionStatus: (status) => set({ connectionStatus: status }),
}));
