import React from 'react';
import { ChatWidget } from '../../components/chat';
import { useSessionStore } from '../../stores/sessionStore';

/** Bare /chat route — for skill apps to iframe the chat widget alone. The
 *  active session is read from the global store (set via session-list
 *  selection or query param). Transport is owned by the entry Root, so the
 *  iframe shares the singleton WebRTC connection. */
export const BareChat: React.FC = () => {
  const sessionId = useSessionStore((s) => s.activeSessionId);
  const projectRoot = useSessionStore((s) => s.selectedProjectRoot);

  return (
    <div className="h-screen w-screen flex flex-col bg-slate-100/70 dark:bg-[#0a0a0a]">
      <ChatWidget sessionId={sessionId} projectRoot={projectRoot} />
    </div>
  );
};
