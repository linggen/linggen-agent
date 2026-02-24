import React, { useEffect, useRef, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import type { MissionInfo } from '../types';

interface MissionSidebarCardProps {
  mission: MissionInfo | null;
  projectRoot: string | null;
  onOpenMission: () => void;
}

export const MissionSidebarCard: React.FC<MissionSidebarCardProps> = ({
  mission,
  projectRoot,
  onOpenMission,
}) => {
  const [missions, setMissions] = useState<MissionInfo[]>([]);
  const endpointAvailable = useRef<boolean | null>(null);
  const prevProjectRoot = useRef(projectRoot);

  // Reset endpoint availability when switching projects
  if (projectRoot !== prevProjectRoot.current) {
    endpointAvailable.current = null;
    prevProjectRoot.current = projectRoot;
  }

  useEffect(() => {
    if (!projectRoot) return;
    if (endpointAvailable.current === false) return;
    const fetchMissions = async () => {
      try {
        const url = new URL('/api/missions', window.location.origin);
        url.searchParams.append('project_root', projectRoot);
        const resp = await fetch(url.toString());
        if (resp.status === 404) {
          endpointAvailable.current = false;
          return;
        }
        endpointAvailable.current = true;
        if (!resp.ok) return;
        const data = await resp.json();
        if (Array.isArray(data?.missions)) {
          setMissions(data.missions.slice(0, 5));
        }
      } catch {
        // silently ignore
      }
    };
    fetchMissions();
  }, [projectRoot, mission]);

  const formatTime = (ts: number) => {
    const d = new Date(ts * 1000);
    return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' }) +
      ', ' +
      d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  };

  const truncate = (text: string, max: number) =>
    text.length > max ? text.slice(0, max).trimEnd() + '…' : text;

  const activeMission = missions.find((m) => m.active);

  return (
    <div className="px-3 py-2 text-xs space-y-2">
      {activeMission ? (
        <p className="text-[11px] text-slate-600 dark:text-slate-300 leading-snug line-clamp-2">
          {activeMission.text}
        </p>
      ) : (
        <p className="text-slate-400 dark:text-slate-500 text-[10px] italic">No active mission</p>
      )}
      <button
        onClick={onOpenMission}
        className="w-full flex items-center justify-center gap-1 px-2 py-1 text-[10px] font-semibold rounded-md text-slate-500 dark:text-slate-400 hover:text-blue-600 dark:hover:text-blue-400 hover:bg-slate-50 dark:hover:bg-white/5 transition-colors"
      >
        {activeMission ? 'Edit' : 'Set Mission'}
        <ChevronRight size={10} />
      </button>
    </div>
  );
};
