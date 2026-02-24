import React from 'react';
import { Settings, Wrench } from 'lucide-react';
import type { SkillInfoFull } from '../types';

const sourceBadgeCls: Record<string, string> = {
  Global: 'bg-purple-500/10 text-purple-600 dark:text-purple-400',
  Project: 'bg-green-500/10 text-green-600 dark:text-green-400',
  Compat: 'bg-slate-500/10 text-slate-600 dark:text-slate-400', // fallback for Compat variants
};

export const SkillsCard: React.FC<{
  skills: SkillInfoFull[];
  onManageSkills: () => void;
}> = ({ skills, onManageSkills }) => {
  if (skills.length === 0) {
    return (
      <div className="p-4 text-center text-[11px] text-slate-400 italic">
        No skills loaded
      </div>
    );
  }

  return (
    <div className="flex-1 p-4 overflow-y-auto text-xs space-y-2">
      {skills.map((skill) => {
        const sourceType = skill.source?.type || 'Global';
        const sourceLabel = sourceType === 'Compat'
          ? (skill.source as { type: string; label?: string })?.label || 'Compat'
          : sourceType === 'Global' ? 'Linggen' : sourceType;
        const trigger = skill.trigger || `/${skill.name}`;
        const argHint = skill.argument_hint ? ` ${skill.argument_hint}` : '';

        return (
          <div
            key={skill.name}
            className="bg-slate-50 dark:bg-black/20 px-3 py-2.5 rounded-xl border border-slate-200 dark:border-white/5"
          >
            <div className="flex items-center justify-between gap-2">
              <span className="font-mono font-bold text-blue-600 dark:text-blue-400 text-[11px]">
                {trigger}
                {argHint && (
                  <span className="text-slate-400 dark:text-slate-500 font-normal">{argHint}</span>
                )}
              </span>
              <span
                className={`text-[9px] font-semibold px-1.5 py-0.5 rounded-full uppercase tracking-wide ${
                  sourceBadgeCls[sourceType] || sourceBadgeCls.Compat
                }`}
              >
                {sourceLabel}
              </span>
            </div>
            {skill.description && (
              <div className="mt-1 text-[10px] text-slate-500 dark:text-slate-400 truncate leading-relaxed">
                {skill.description}
              </div>
            )}
            {skill.tool_defs.length > 0 && (
              <div className="mt-1 text-[10px] text-slate-400 dark:text-slate-500 flex items-center gap-1">
                <Wrench size={9} />
                {skill.tool_defs.length} tool{skill.tool_defs.length !== 1 ? 's' : ''}
              </div>
            )}
          </div>
        );
      })}

      <button
        type="button"
        onClick={onManageSkills}
        className="w-full mt-2 inline-flex items-center justify-center gap-1.5 px-3 py-1.5 text-[10px] font-semibold text-slate-500 dark:text-slate-400 hover:text-blue-600 dark:hover:text-blue-400 hover:bg-slate-50 dark:hover:bg-white/5 rounded-lg transition-colors"
      >
        <Settings size={11} />
        Manage Skills
      </button>
    </div>
  );
};
