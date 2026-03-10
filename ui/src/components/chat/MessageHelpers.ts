import type { ChatMessage } from '../../types';
import { sanitizeAgentMessageText } from './utils/message';

export const statusBadgeClass = (status?: string) => {
  if (status === 'working') return 'bg-green-500/15 text-green-600 dark:text-green-300';
  if (status === 'thinking') return 'bg-blue-500/15 text-blue-600 dark:text-blue-300';
  if (status === 'calling_tool') return 'bg-amber-500/15 text-amber-700 dark:text-amber-300';
  if (status === 'model_loading') return 'bg-indigo-500/15 text-indigo-700 dark:text-indigo-300';
  return 'bg-slate-500/15 text-slate-600 dark:text-slate-300';
};

export const hasReadFileActivity = (entries?: string[]) =>
  Array.isArray(entries) &&
  entries.some((entry) => {
    const t = String(entry || '').trim();
    return /^Calling tool:\s*read\b/i.test(t) || /^Reading file(?::|\.\.\.)/i.test(t);
  });

export const looksLikeFileDump = (text: string) => {
  const lines = text.split('\n');
  if (lines.length < 40) return false;
  const codeish = lines.filter((line) =>
    /^\s*(\/\/|#include|use\s+\w|import\s+\w|fn\s+\w|class\s+\w|def\s+\w|const\s+\w|let\s+\w|pub\s+\w|[{}();]|<\/?\w+)/.test(
      line
    )
  ).length;
  return codeish >= Math.min(25, Math.floor(lines.length * 0.4));
};

export const redactFileDumpForReadFile = (text: string) => {
  let changed = false;
  const redactedBlocks = text.replace(/```[\s\S]*?```/g, (block) => {
    const blockLines = block.split('\n').length;
    if (blockLines < 12) return block;
    changed = true;
    return '```text\n[file content omitted]\n```';
  });
  if (changed) return redactedBlocks;
  if (looksLikeFileDump(text)) return '[file content omitted]';
  return text;
};

export const visibleMessageText = (msg: ChatMessage) => {
  if (msg.role === 'user') return msg.text;
  const sanitized = sanitizeAgentMessageText(msg.text);
  const cleaned = sanitized.split('\n')
    .filter(line => {
      const t = line.trimStart();
      return !t.startsWith('Used tool:') &&
             !t.startsWith('Tool done:') &&
             !t.startsWith('Tool tool_not_allowed:') &&
             !t.startsWith('Tool error:') &&
             !t.startsWith('tool_not_allowed:') &&
             !t.startsWith('Delegated task:');
    })
    .join('\n').replace(/\n{3,}/g, '\n\n').trim();
  if (!hasReadFileActivity(msg.activityEntries)) return cleaned;
  return redactFileDumpForReadFile(cleaned);
};
