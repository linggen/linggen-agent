import React from 'react';
import DiffView, { diffStats } from '../DiffView';
import { MarkdownContent } from './MarkdownContent';
import { PlanBlock } from './PlanBlock';
import { PLAN_STATUS_COLOR, PLAN_ITEM_ICON, PLAN_ITEM_COLOR } from './utils/plan';
import type { SpecialBlockProps } from './types';

export function tryRenderSpecialBlock(
  text: string,
  props: SpecialBlockProps,
): React.ReactNode | null {
  try {
    const parsed = JSON.parse(text);

    if (parsed.type === 'plan' && parsed.plan) {
      return (
        <PlanBlock
          plan={parsed.plan}
          statusColor={PLAN_STATUS_COLOR}
          itemIcon={PLAN_ITEM_ICON}
          itemColor={PLAN_ITEM_COLOR}
          pendingPlanAgentId={props.pendingPlanAgentId}
          agentContext={props.agentContext}
          onApprovePlan={props.onApprovePlan}
          onRejectPlan={props.onRejectPlan}
          onEditPlan={props.onEditPlan}
          inputRef={props.inputRef}
        />
      );
    }

    if (parsed.type === 'finalize_task' && parsed.packet) {
      const packet = parsed.packet;
      const userStories: string[] = Array.isArray(packet.user_stories) ? packet.user_stories : [];
      const criteria: string[] = Array.isArray(packet.acceptance_criteria)
        ? packet.acceptance_criteria
        : [];
      return (
        <div className="space-y-2">
          <div className="font-bold text-blue-500">Task Finalized: {packet.title}</div>
          {userStories.length > 0 && (
            <div className="space-y-1 text-[11px]">
              <div className="uppercase tracking-wider text-[9px] text-slate-500">User Stories</div>
              {userStories.map((story: string, idx: number) => (
                <div key={idx} className="text-[11px] opacity-90">- {story}</div>
              ))}
            </div>
          )}
          {criteria.length > 0 && (
            <div className="space-y-1 text-[11px]">
              <div className="uppercase tracking-wider text-[9px] text-slate-500">Acceptance Criteria</div>
              {criteria.map((crit: string, idx: number) => (
                <div key={idx} className="text-[11px] opacity-90">- {crit}</div>
              ))}
            </div>
          )}
        </div>
      );
    }

    if (parsed.type === 'change_report' && Array.isArray(parsed.files)) {
      const files = parsed.files
        .map((item: any) => ({
          path: typeof item?.path === 'string' ? item.path : '',
          summary: typeof item?.summary === 'string' ? item.summary : '',
          diff: typeof item?.diff === 'string' ? item.diff : '',
        }))
        .filter((item: any) => item.path);
      const truncatedCount = Number(parsed.truncated_count || 0);
      const reviewHint = typeof parsed.review_hint === 'string' ? parsed.review_hint : '';
      return (
        <div className="space-y-1">
          <div className="font-bold text-blue-500">
            Changed files ({files.length}
            {truncatedCount > 0 ? ` +${truncatedCount} more` : ''})
          </div>
          {files.map((file: any, idx: number) => {
            const hasDiff = !!file.diff && !file.diff.startsWith('(diff');
            const stats = hasDiff ? diffStats(file.diff) : null;
            if (!hasDiff) {
              return (
                <div
                  key={`${file.path}-${idx}`}
                  className="flex flex-wrap items-center gap-2 rounded-md border border-slate-200 dark:border-white/10 bg-slate-50/80 dark:bg-white/[0.03] px-2 py-1.5 text-[11px]"
                >
                  <span className="text-slate-500 dark:text-slate-300">{file.summary || 'Updated'}</span>
                  <span className="font-mono text-[11px]">{file.path}</span>
                </div>
              );
            }
            return (
              <details
                key={`${file.path}-${idx}`}
                className="rounded-md border border-slate-200 dark:border-white/10 bg-slate-50/80 dark:bg-white/[0.03] text-[11px]"
              >
                <summary className="cursor-pointer px-2 py-1.5 select-none flex flex-wrap items-center gap-2">
                  <span className="text-slate-500 dark:text-slate-300">{file.summary || 'Updated'}</span>
                  <span className="font-mono text-[11px]">{file.path}</span>
                  {stats && (
                    <span className="ml-auto font-mono text-[10px]">
                      {stats.added > 0 && <span className="text-green-600 dark:text-green-400">+{stats.added}</span>}
                      {stats.added > 0 && stats.deleted > 0 && ' '}
                      {stats.deleted > 0 && <span className="text-red-600 dark:text-red-400">-{stats.deleted}</span>}
                    </span>
                  )}
                </summary>
                <div className="px-1 pb-1">
                  <DiffView diff={file.diff} />
                </div>
              </details>
            );
          })}
          {reviewHint && (
            <div className="text-[11px] text-slate-500 dark:text-slate-400">{reviewHint}</div>
          )}
        </div>
      );
    }

    // JSON with text-like field — extract and render as markdown
    const textContent =
      typeof parsed.response === 'string' ? parsed.response
      : typeof parsed.text === 'string' ? parsed.text
      : typeof parsed.content === 'string' ? parsed.content
      : typeof parsed.message === 'string' ? parsed.message
      : typeof parsed.answer === 'string' ? parsed.answer
      : null;
    if (textContent) return <MarkdownContent text={textContent} />;
  } catch { /* not JSON */ }
  return null;
}
