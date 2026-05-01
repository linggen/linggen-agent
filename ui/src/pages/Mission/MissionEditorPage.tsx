import React from 'react';
import { useNavigate } from 'react-router-dom';
import { MissionEditor } from '../../components/MissionPage';
import { useUiStore } from '../../stores/uiStore';
import { useSessionStore } from '../../stores/sessionStore';

/** Full-screen mission editor at /missions/edit. Reads the mission to edit
 *  from `uiStore.editingMission` (set by `useOpenMissionEditor`). On save or
 *  cancel, returns to the home route. */
export const MissionEditorPage: React.FC = () => {
  const navigate = useNavigate();
  const editingMission = useUiStore((s) => s.editingMission);
  const closeMissionEditor = useUiStore((s) => s.closeMissionEditor);
  const bumpMissionRefreshKey = useUiStore((s) => s.bumpMissionRefreshKey);
  const allSessions = useSessionStore((s) => s.allSessions);

  const workingFolders = React.useMemo(() => {
    const folders = new Set<string>();
    for (const s of allSessions) {
      if (s.cwd) folders.add(s.cwd);
    }
    return [...folders].sort();
  }, [allSessions]);

  const close = () => {
    closeMissionEditor();
    navigate('/');
  };

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200 font-sans overflow-hidden">
      <MissionEditor
        editing={editingMission}
        workingFolders={workingFolders}
        onSave={() => { bumpMissionRefreshKey(); close(); }}
        onCancel={close}
        onViewAgent={() => {}}
      />
    </div>
  );
};
