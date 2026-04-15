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
  userName: string | null;
  avatarUrl: string | null;
  userPermission: 'admin' | 'edit' | 'read' | 'chat' | 'pending';
  userRoomName: string | null;
  userTokenBudget: number | null;
  /** Whether owner's room is enabled (accepting consumers). */
  roomEnabled: boolean;
  connectionStatus: 'connected' | 'reconnecting' | 'disconnected';
  /** Role in a room: 'owner' if sharing, 'consumer' if joined via proxy. */
  roomRole: 'owner' | 'consumer' | null;
  /** Name of the proxy room the user joined (consumer role). */
  proxyRoomName: string | null;

  setUserType: (userType: 'owner' | 'consumer') => void;
  setUserId: (userId: string) => void;
  setUserProfile: (name: string | null, avatar: string | null) => void;
  setUserInfo: (permission: string, roomName?: string | null, tokenBudget?: number | null) => void;
  setRoomEnabled: (enabled: boolean) => void;
  setConnectionStatus: (status: 'connected' | 'reconnecting' | 'disconnected') => void;
  setProxyRoom: (roomName: string | null) => void;
}

const isRemote = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

export const useUserStore = create<UserState>((set) => ({
  userType: isRemote ? 'consumer' : 'owner',
  userId: null,
  userName: null,
  avatarUrl: null,
  userPermission: isRemote ? 'pending' as any : 'admin',
  userRoomName: null,
  userTokenBudget: null,
  roomEnabled: true,
  connectionStatus: isRemote ? 'disconnected' : 'connected',
  roomRole: null,
  proxyRoomName: null,

  setUserType: (userType) => set({ userType }),
  setUserId: (userId) => set({ userId }),
  setUserProfile: (name, avatar) => set({ userName: name, avatarUrl: avatar }),
  setUserInfo: (permission, roomName, tokenBudget) => set({
    userPermission: permission as any,
    userRoomName: roomName ?? null,
    userTokenBudget: tokenBudget ?? null,
  }),
  setRoomEnabled: (enabled) => set({ roomEnabled: enabled }),
  setConnectionStatus: (status) => set({ connectionStatus: status }),
  setProxyRoom: (roomName) => set({ proxyRoomName: roomName, roomRole: roomName ? 'consumer' : null }),
}));
