/**
 * Main entry — owner's full UI: sidebar, chat, info panel, settings, missions.
 * Uses responsive CSS for mobile (<768px) — no separate mobile entry.
 */
import React, { useState, useMemo, useEffect, useRef, useCallback } from 'react';
import { X } from 'lucide-react';
import { SessionList } from '../components/SessionList';
import { FilePreview } from '../components/FilePreview';
import { ChatWidget } from '../components/chat';
import { HeaderBar } from '../components/HeaderBar';
import { SettingsPage } from '../components/SettingsPage';
import { MissionEditor } from '../components/MissionPage';
import { AgentSpecEditorModal } from '../components/AgentSpecEditorModal';
import { ToastContainer } from '../components/ToastContainer';
import { AppPanel } from '../components/AppPanel';
import { InfoPanel } from '../components/InfoPanel';
import { RoomChatPanel } from '../components/RoomChatPanel';
import { recordSkillUsage } from '../components/SkillsCard';
import { buildAgentWorkInfo } from '../lib/messageUtils';
import { useSessionStore } from '../stores/sessionStore';
import { useServerStore } from '../stores/serverStore';
import { useChatStore } from '../stores/chatStore';
import { useUiStore } from '../stores/uiStore';
import { useUserStore } from '../stores/userStore';
import { useInteractionStore } from '../stores/interactionStore';
import { useRunInfo } from '../hooks/useRunInfo';
import { useChatActions } from '../hooks/useChatActions';
import { useTransport, sendViewContext } from '../hooks/useTransport';

const urlParams = new URLSearchParams(window.location.search);
const isMobileParam = urlParams.get('mode') === 'mobile';

/** Detect mobile viewport (< 768px) or explicit ?mode=mobile. */
function useIsMobile(): boolean {
  const [mobile, setMobile] = useState(
    isMobileParam || window.innerWidth < 768,
  );
  useEffect(() => {
    if (isMobileParam) return;
    const mq = window.matchMedia('(max-width: 767px)');
    const onChange = (e: MediaQueryListEvent) => setMobile(e.matches);
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, []);
  return mobile;
}

/** Detect remote/tunnel mode (blob iframe with injected instance meta tag). */
const isRemoteMode = typeof document !== 'undefined' && !!document.querySelector('meta[name="linggen-instance"]');

export const MainApp: React.FC = () => {
  // --- Stores ---
  const projectStore = useSessionStore();
  const agentStore = useServerStore();
  const chatStore = useChatStore();
  const uiStore = useUiStore();

  // Initialize transport — must run before connection gate to establish WebRTC.
  useTransport({ sessionId: projectStore.activeSessionId });

  const isMobile = useIsMobile();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const [mobileInfoOpen, setMobileInfoOpen] = useState(false);

  // Shortcuts
  const { selectedProjectRoot, sessions, activeSessionId, isMissionSession } = projectStore;
  const { agents, models, skills, selectedAgent, agentStatus, agentStatusText, defaultModels, ollamaStatus, reloadingSkills, agentTreesByProject } = agentStore;
  const { messages: chatMessages } = chatStore;
  const { currentPage, editingMission, showAgentSpecEditor, openApp, selectedFileContent, selectedFilePath } = uiStore;

  const isRunning = agentStore.isRunning();
  const mainAgents = agents;

  // Session-change tracking (for clear-on-switch)
  const prevSessionIdRef = useRef<string | null>(null);

  // --- Derived memos (sidebar only) ---
  const activeModelId = useMemo(() => {
    if (!activeSessionId) return undefined;
    const status = agentStatus[activeSessionId];
    if (status && status !== 'idle') {
      const text = agentStatusText[activeSessionId] || '';
      const match = text.match(/\(([^)]+)\)/);
      if (match) return match[1];
    }
    return undefined;
  }, [agentStatus, agentStatusText, activeSessionId]);

  const mainAgentIds = useMemo(() => agents.map((a) => a.name.toLowerCase()), [agents]);

  const agentTree = useMemo(
    () => agentTreesByProject[selectedProjectRoot] || {},
    [agentTreesByProject, selectedProjectRoot],
  );
  // agentWork is derived here for future use by the sidebar; currently not wired.
  useMemo(() => buildAgentWorkInfo(agentTree), [agentTree]);

  // --- Run info ---
  const { runningMainRunIds } = useRunInfo();

  // --- Chat actions ---
  const scrollToBottomNoop = useCallback(() => {}, []);
  const { sendChatMessage } = useChatActions(scrollToBottomNoop, runningMainRunIds);

  // --- Initial data load ---
  // Most data is delivered via server-pushed page_state after WebRTC connects.
  // Only fetch data not covered by page_state, or needed before WebRTC is ready.
  useEffect(() => {
    useServerStore.getState().fetchOllamaStatus();
    useServerStore.getState().fetchSessionTokens();
  }, []);

  // --- React to selected project changes ---
  useEffect(() => {
    if (selectedProjectRoot) {
      useChatStore.getState().fetchSessionState();
      sendViewContext();
      useInteractionStore.getState().setQueuedMessages([]);
      useInteractionStore.getState().setActivePlan(null);
    }
  }, [selectedProjectRoot]);

  // --- React to session changes ---
  useEffect(() => {
    const { isSkillSession } = projectStore;
    if (selectedProjectRoot || isMissionSession || isSkillSession || activeSessionId) {
      const prev = prevSessionIdRef.current;
      prevSessionIdRef.current = activeSessionId;
      const isSessionAdoption = prev === null && activeSessionId !== null;

      // Switch to the session's message bucket — instant if cached, no clear needed
      const cs = useChatStore.getState();
      cs.setActiveSession(activeSessionId);

      if (!isSessionAdoption) {
        const interaction = useInteractionStore.getState();
        interaction.setQueuedMessages([]);
        interaction.setActivePlan(null);
        interaction.setPendingPlan(null);
        interaction.setPendingPlanAgentId(null);
        const prevSid = prevSessionIdRef.current;
        if (prevSid) {
          useServerStore.getState().setAgentStatus((prev) => { const n = { ...prev }; delete n[prevSid]; return n; });
          useServerStore.getState().setAgentStatusText((prev) => { const n = { ...prev }; delete n[prevSid]; return n; });
        }
      }
      cs.fetchSessionState();
      sendViewContext();
    }
  }, [activeSessionId, selectedProjectRoot, isMissionSession, projectStore.isSkillSession]);

  // --- Restore persisted session-level model override ---
  useEffect(() => {
    const sess = sessions.find((s) => s.id === activeSessionId);
    useUiStore.getState().setSessionModel(sess?.model_id ?? null);
  }, [activeSessionId, sessions]);

  // --- Poll workspace state for mission sessions (backup; events also trigger reloads) ---
  useEffect(() => {
    if (!isMissionSession || !activeSessionId) return;
    const interval = setInterval(() => {
      useChatStore.getState().fetchSessionState();
    }, 5000);
    return () => clearInterval(interval);
  }, [isMissionSession, activeSessionId]);

  // --- Auto-select agent ---
  useEffect(() => {
    if (mainAgentIds.length === 0) return;
    if (!mainAgentIds.includes(selectedAgent.toLowerCase())) {
      const preferred = mainAgentIds.includes('ling') ? 'ling' : mainAgentIds[0];
      useServerStore.getState().setSelectedAgent(preferred);
    }
  }, [mainAgentIds, selectedAgent]);

  // --- Mobile: set data attribute for CSS safe-area insets ---
  useEffect(() => {
    if (isMobile) {
      document.documentElement.setAttribute('data-mobile', '');
    } else {
      document.documentElement.removeAttribute('data-mobile');
    }
  }, [isMobile]);

  // --- Sidebar callbacks ---
  const readFile = useCallback(async (path: string, projectRootOverride?: string) => {
    const root = projectRootOverride || useSessionStore.getState().selectedProjectRoot;
    if (!root) return;
    try {
      const resp = await fetch(`/api/file?project_root=${encodeURIComponent(root)}&path=${encodeURIComponent(path)}`);
      const data = await resp.json();
      useUiStore.getState().setSelectedFileContent(data.content);
      useUiStore.getState().setSelectedFilePath(path);
    } catch (e) { console.error('Error reading file:', e); }
  }, []);

  // --- Info panel props (shared between desktop sidebar and mobile drawer) ---
  const handleClickSkill = useCallback((skill: any) => {
    recordSkillUsage(skill.name);
    if (skill.app) {
      if (skill.app.launcher === 'web') {
        const appUrl = `/apps/${skill.name}/${skill.app.entry}`;
        if (isRemoteMode) {
          const instanceId = document.querySelector('meta[name="linggen-instance"]')?.getAttribute('content') || '';
          const relayOrigin = document.querySelector('meta[name="linggen-relay-origin"]')?.getAttribute('content') || '';
          window.open(`${relayOrigin}/app/connect/${instanceId}?app=${encodeURIComponent(appUrl)}`, '_blank');
        } else {
          window.open(appUrl, '_blank');
        }
      } else if (skill.app.launcher === 'url') {
        window.open(skill.app.entry, '_blank');
      } else {
        sendChatMessage(`/${skill.name} --web`);
      }
    } else sendChatMessage(`/${skill.name}`);
  }, [sendChatMessage]);

  const infoPanelProps = {
    models, skills, agents: mainAgents, chatMessages, activeModelId,
    defaultModels, ollamaStatus, reloadingSkills,
    projectRoot: selectedProjectRoot,
    onToggleDefault: agentStore.toggleDefaultModel,
    onChangeReasoningEffort: agentStore.setReasoningEffort,
    onReloadSkills: () => agentStore.reloadSkills(),
    onOpenSettings: (tab: string) => uiStore.openSettings(tab),
    onClickSkill: handleClickSkill,
  };

  // --- Remote mode: gate rendering until transport is connected ---
  const connectionStatus = useUserStore((s) => s.connectionStatus);
  if (isRemoteMode && connectionStatus === 'disconnected') {
    return (
      <div className="h-screen flex items-center justify-center bg-white dark:bg-[#0b0e14]">
        <div className="text-center space-y-4">
          <div className="inline-block animate-spin rounded-full h-10 w-10 border-4 border-slate-200 dark:border-slate-700 border-t-emerald-500" />
          <p className="text-slate-500 dark:text-slate-400 text-sm">Connecting to linggen server…</p>
        </div>
      </div>
    );
  }

  const selectSession = (session: any, closeMenu?: () => void) => {
    const isMission = session.creator === 'mission';
    const isSkill = session.creator === 'skill' || (!session.project && session.skill);
    projectStore.setActiveSessionId(session.id);
    projectStore.setIsMissionSession(isMission);
    projectStore.setIsSkillSession(!!isSkill);
    projectStore.setActiveSkillName(isSkill && session.skill ? session.skill : null);
    projectStore.setActiveMissionId(isMission && session.mission_id ? session.mission_id : null);
    const sessionRoot = session.project || session.cwd || session.repo_path || '~';
    if (sessionRoot !== selectedProjectRoot) {
      projectStore.setSelectedProjectRoot(sessionRoot);
    }
    window.localStorage.setItem('linggen:active-session', session.id);
    closeMenu?.();
  };

  // --- Render ---
  return (
    <>
      <ToastContainer />
      {currentPage === 'mission-editor' && (
        <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
          <MissionEditor
            editing={editingMission}
            onSave={() => { uiStore.bumpMissionRefreshKey(); uiStore.closeMissionEditor(); }}
            onCancel={() => uiStore.closeMissionEditor()}
            onViewAgent={() => {}}
          />
        </div>
      )}
      {currentPage === 'settings' && (
        <SettingsPage
          onBack={() => {
            uiStore.setCurrentPage('main');
            uiStore.setInitialSettingsTab(undefined);
            agentStore.fetchModels();
            agentStore.fetchDefaultModels();
            agentStore.fetchOllamaStatus();
          }}
          projectRoot={selectedProjectRoot}
          initialTab={uiStore.initialSettingsTab}
          missionAgents={agents}
        />
      )}
      <div className={`flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden${currentPage !== 'main' ? ' hidden' : ''}`}>
        {/* Header */}
        <HeaderBar
          isRunning={isRunning}
          onOpenSettings={() => uiStore.setCurrentPage('settings')}
          onToggleMobileMenu={() => setMobileMenuOpen(!mobileMenuOpen)}
          onToggleInfoPanel={isMobile ? () => setMobileInfoOpen(!mobileInfoOpen) : undefined}
        />

        {/* Main Layout */}
        <div className="flex-1 flex overflow-hidden">

          {/* Mobile slide-over session list */}
          {mobileMenuOpen && (
            <>
              <div className="fixed inset-0 bg-black/30 z-40 md:hidden" onClick={() => setMobileMenuOpen(false)} />
              <div className="fixed inset-y-0 left-0 w-72 z-50 md:hidden bg-white dark:bg-[#0f0f0f] shadow-xl animate-slide-in flex flex-col">
                <SessionList
                  activeSessionId={activeSessionId}
                  onSelectSession={(session) => selectSession(session, () => setMobileMenuOpen(false))}
                  onCreateSession={() => { projectStore.createSession(); setMobileMenuOpen(false); }}
                  onDeleteSession={(id) => projectStore.removeSession(id)}
                  onOpenSettings={(tab) => { uiStore.openSettings(tab as any); setMobileMenuOpen(false); }}
                />
              </div>
            </>
          )}

          {/* Left sidebar — unified session list (desktop only) */}
          <div className="hidden md:flex w-72 border-r border-slate-200 dark:border-white/5 flex-col bg-white dark:bg-[#0f0f0f] h-full">
            <SessionList
              activeSessionId={activeSessionId}
              onSelectSession={(session) => selectSession(session)}
              onCreateSession={() => projectStore.createSession()}
              onDeleteSession={(id) => projectStore.removeSession(id)}
              onOpenSettings={(tab) => uiStore.openSettings(tab as any)}
            />
            <RoomChatPanel />
          </div>

          {/* Center: Chat */}
          <main className={`flex-1 flex flex-col overflow-hidden bg-slate-100/40 dark:bg-[#0a0a0a] min-h-0${isMobile ? ' p-0' : ''}`}>
            <div className={`flex-1 min-h-0${isMobile ? '' : ' p-2'}`}>
              <ChatWidget
                sessionId={activeSessionId}
                projectRoot={selectedProjectRoot}
                mode={isMobile ? 'mobile' : 'full'}
              />
            </div>
          </main>

          {/* Right sidebar (desktop only) */}
          {!isMobile && (
            <aside className="hidden lg:flex w-72 border-l border-slate-200 dark:border-white/5 flex-col bg-slate-100/40 dark:bg-[#0a0a0a] p-3 gap-3 overflow-y-auto">
              <InfoPanel {...infoPanelProps} />
            </aside>
          )}

          {/* Mobile right drawer (models + skills) */}
          {isMobile && mobileInfoOpen && (
            <>
              <div className="fixed inset-0 bg-black/30 z-40" onClick={() => setMobileInfoOpen(false)} />
              <div className="fixed inset-y-0 right-0 w-72 z-50 bg-white dark:bg-[#0f0f0f] shadow-xl flex flex-col animate-slide-in-right">
                <div className="flex items-center justify-between px-3 py-2 border-b border-slate-200 dark:border-white/10">
                  <span className="text-xs font-bold uppercase tracking-widest text-slate-500">Info</span>
                  <button onClick={() => setMobileInfoOpen(false)} className="p-1 rounded hover:bg-slate-100 dark:hover:bg-white/5 text-slate-400">
                    <X size={16} />
                  </button>
                </div>
                <div className="flex-1 overflow-y-auto p-3 space-y-3">
                  <InfoPanel {...infoPanelProps} />
                </div>
              </div>
            </>
          )}
        </div>

        <FilePreview selectedFilePath={selectedFilePath} selectedFileContent={selectedFileContent} onClose={() => uiStore.closeFilePreview()} />
        <AgentSpecEditorModal open={showAgentSpecEditor} projectRoot={selectedProjectRoot}
          onClose={() => uiStore.setShowAgentSpecEditor(false)}
          onChanged={() => { agentStore.fetchAgents(selectedProjectRoot); chatStore.fetchSessionState(); }} />

        {openApp && <AppPanel app={openApp} onClose={() => uiStore.setOpenApp(null)} />}

        <style>{`
          .custom-scrollbar { scrollbar-gutter: stable; }
          .custom-scrollbar::-webkit-scrollbar { width: 8px; }
          .custom-scrollbar::-webkit-scrollbar-track { background: rgba(0, 0, 0, 0.04); }
          .custom-scrollbar::-webkit-scrollbar-thumb { background: rgba(59, 130, 246, 0.45); border-radius: 10px; }
          .custom-scrollbar::-webkit-scrollbar-thumb:hover { background: rgba(59, 130, 246, 0.7); }
        `}</style>
      </div>
    </>
  );
};
