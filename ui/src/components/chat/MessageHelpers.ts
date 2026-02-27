import type { ChatMessage } from '../../types';
import { sanitizeAgentMessageText } from './utils/message';

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
