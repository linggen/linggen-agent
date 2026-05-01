import React from 'react';
import { SessionList } from '../../components/SessionList';
import { useSessionStore } from '../../stores/sessionStore';
import { useOpenSettings } from '../../hooks/useOpenSettings';

/** Bare /sessions route — for skill apps to iframe the session list alone.
 *  Selection writes to the global session store; consuming widgets in the
 *  same iframe (or postMessage subscribers) react to the change. Transport
 *  is owned by the entry Root. */
export const BareSessions: React.FC = () => {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const sessionStore = useSessionStore();
  const openSettings = useOpenSettings();

  return (
    <div className="h-screen w-screen bg-white dark:bg-[#0f0f0f] flex flex-col">
      <SessionList
        activeSessionId={activeSessionId}
        onSelectSession={(session) => {
          sessionStore.setActiveSessionId(session.id);
          window.localStorage.setItem('linggen:active-session', session.id);
        }}
        onCreateSession={() => sessionStore.createSession()}
        onDeleteSession={(id) => sessionStore.removeSession(id)}
        onOpenSettings={(tab) => openSettings(tab as any)}
      />
    </div>
  );
};
