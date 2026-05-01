import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import { useUiStore } from '../stores/uiStore';
import type { CronMission } from '../types';

/** Open the mission editor at /missions/edit, optionally with a mission to edit. */
export function useOpenMissionEditor() {
  const navigate = useNavigate();
  return useCallback((mission: CronMission | null) => {
    useUiStore.getState().openMissionEditor(mission);
    navigate('/missions/edit');
  }, [navigate]);
}
