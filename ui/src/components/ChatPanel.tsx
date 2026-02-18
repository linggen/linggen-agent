import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { Send, X, Sparkles } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { cn } from '../lib/cn';
import type {
  AgentInfo,
  AgentRunInfo,
  AgentRunContextMessage,
  AgentRunContextResponse,
  ChatMessage,
  QueuedChatItem,
  SkillInfo,
  SubagentInfo,
} from '../types';

let mermaidInstance: any = null;
let mermaidInitialized = false;

async function getMermaid() {
  if (!mermaidInstance) {
    const module = await import('mermaid');
    mermaidInstance = module.default;
  }
  if (!mermaidInitialized) {
    mermaidInstance.initialize({
      startOnLoad: false,
      securityLevel: 'strict',
      theme: 'default',
    });
    mermaidInitialized = true;
  }
  return mermaidInstance;
}

const hashText = (text: string) => {
  let hash = 0;
  for (let i = 0; i < text.length; i += 1) {
    hash = (hash * 31 + text.charCodeAt(i)) | 0;
  }
  return Math.abs(hash).toString(36);
};

const MermaidBlock: React.FC<{ code: string }> = ({ code }) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const uniqueId = useId().replace(/:/g, '');
  const idRef = useRef(`chat-mermaid-${hashText(code)}-${uniqueId}`);

  useEffect(() => {
    let cancelled = false;

    const render = async () => {
      setError(null);
      if (!containerRef.current) return;
      containerRef.current.innerHTML = '<div class="markdown-mermaid-loading">Rendering Mermaid...</div>';
      try {
        const mermaid = await getMermaid();
        const { svg } = await mermaid.render(idRef.current, code.trim());
        if (!cancelled && containerRef.current) {
          containerRef.current.innerHTML = svg;
        }
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    };

    render();
    return () => {
      cancelled = true;
    };
  }, [code]);

  if (error) {
    return (
      <div className="markdown-mermaid-error">
        Mermaid error: {error}
      </div>
    );
  }
  return <div className="markdown-mermaid" ref={containerRef} />;
};

const MarkdownContent: React.FC<{ text: string }> = ({ text }) => (
  <div className="markdown-body break-words">
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        pre: ({ children }) => <>{children}</>,
        code: ({ inline, className, children, node: _node, ...props }: any) => {
          const raw = String(children ?? '').replace(/\n$/, '');
          const match = /language-([\w-]+)/.exec(className || '');
          const lang = match?.[1]?.toLowerCase();
          if (!inline && lang === 'mermaid') {
            return <MermaidBlock code={raw} />;
          }
          const isInlineCode = Boolean(inline) || (!className && !raw.includes('\n'));
          if (isInlineCode) {
            return <code {...props}>{children}</code>;
          }
          return (
            <pre>
              <code className={className} {...props}>{raw}</code>
            </pre>
          );
        },
      }}
    >
      {normalizeMarkdownish(text)}
    </ReactMarkdown>
  </div>
);

function normalizeMarkdownish(text: string): string {
  // Improve readability when model emits markdown tokens without proper newlines.
  return text
    .replace(/\s+(#{1,6}\s)/g, '\n\n$1')
    .replace(/\s+(\d+\.\s)/g, '\n$1')
    .replace(/\s+(-\s)/g, '\n$1')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

const normalizeAgentKey = (value?: string) => (value || '').trim().toLowerCase();
const normalizeMessageTextForDedup = (text?: string) =>
  (text || '').replace(/\s+/g, ' ').trim();

const sortMessagesByTime = (messages: ChatMessage[]) =>
  messages
    .map((msg, index) => ({ msg, index }))
    .sort((a, b) => {
      const ta = a.msg.timestampMs ?? 0;
      const tb = b.msg.timestampMs ?? 0;
      if (ta <= 0 && tb <= 0) return a.index - b.index;
      if (ta <= 0) return 1;
      if (tb <= 0) return -1;
      if (ta !== tb) return ta - tb;
      return a.index - b.index;
    })
    .map((entry) => entry.msg);

const hasStrongContentOverlap = (aText: string, bText: string) => {
  if (!aText || !bText) return false;
  if (aText === bText) return true;
  const [shorter, longer] =
    aText.length <= bText.length ? [aText, bText] : [bText, aText];
  // Avoid over-merging short generic messages like "yes", "ok", etc.
  if (shorter.length < 80) return false;
  if (!longer.includes(shorter)) return false;
  return shorter.length / longer.length >= 0.45;
};

const statusBadgeClass = (status?: string) => {
  if (status === 'working') return 'bg-green-500/15 text-green-600 dark:text-green-300';
  if (status === 'thinking') return 'bg-blue-500/15 text-blue-600 dark:text-blue-300';
  if (status === 'calling_tool') return 'bg-amber-500/15 text-amber-700 dark:text-amber-300';
  if (status === 'model_loading') return 'bg-indigo-500/15 text-indigo-700 dark:text-indigo-300';
  return 'bg-slate-500/15 text-slate-600 dark:text-slate-300';
};

const roleFromSender = (sender: string): ChatMessage['role'] => {
  const key = normalizeAgentKey(sender);
  if (key === 'user') return 'user';
  return 'agent';
};

const TOOL_JSON_EMBEDDED_RE = /\{"type":"tool","tool":"([^"]+)","args":\{[\s\S]*?\}\}/g;
const TOOL_RESULT_LINE_RE = /^(Tool\s+[A-Za-z0-9_.:-]+\s*:|tool_error:|tool_not_allowed:)/i;
const START_AUTONOMOUS_LINE_RE = /^Starting autonomous loop for task:/i;
const CONTENT_OMITTED_LINE_RE = /^\(content omitted in chat; open the file viewer for full text\)$/i;

const parseToolNameFromLine = (line: string): string | null => {
  const trimmed = line.trim();
  if (!trimmed.startsWith('{')) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed?.type === 'tool' && typeof parsed?.tool === 'string') return parsed.tool;
    if (
      typeof parsed?.type === 'string' &&
      parsed.type !== 'finalize_task' &&
      parsed.args &&
      typeof parsed.args === 'object'
    ) {
      return parsed.type;
    }
  } catch (_e) {
    // ignore non-json
  }
  return null;
};

const looksLikeCodeLine = (line: string) =>
  /^\s*(\/\/|#include|use\s+\w|import\s+\w|fn\s+\w|class\s+\w|def\s+\w|const\s+\w|let\s+\w|pub\s+\w|impl\s+\w|struct\s+\w|enum\s+\w|mod\s+\w|[{}[\]();]|<\/?\w+|[A-Za-z_][A-Za-z0-9_]*::[A-Za-z_])/.test(
    line
  );

type RenderChunk =
  | { type: 'text'; text: string }
  | { type: 'bash'; exitCode?: string; stdout: string; stderr: string };

const BASH_OUTPUT_HEADER_RE = /^Bash output \(exit_code:\s*([^)]+)\):\s*$/i;
const TOOL_BASH_OUTPUT_HEADER_RE =
  /^Tool\s+Bash\s*:\s*Bash output \(exit_code:\s*([^)]+)\):\s*$/i;

const trimTrailingEmptyLines = (lines: string[]) => {
  const out = [...lines];
  while (out.length > 0 && out[out.length - 1].trim() === '') out.pop();
  return out;
};

const normalizeExitCode = (raw?: string) => {
  const value = String(raw || '').trim();
  if (!value) return undefined;
  const someMatch = /^Some\(([-+]?\d+)\)$/.exec(value);
  if (someMatch?.[1]) return someMatch[1];
  if (value === 'None') return 'n/a';
  return value;
};

const parseBashOutputChunks = (text: string): RenderChunk[] => {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const chunks: RenderChunk[] = [];
  let plainBuffer: string[] = [];
  let i = 0;

  const flushText = () => {
    if (plainBuffer.length === 0) return;
    const chunk = plainBuffer.join('\n').trim();
    if (chunk) chunks.push({ type: 'text', text: chunk });
    plainBuffer = [];
  };

  while (i < lines.length) {
    const line = lines[i] || '';
    const trimmed = line.trim();
    let exitCode: string | undefined;
    let atStdout = false;

    const headerMatch = BASH_OUTPUT_HEADER_RE.exec(trimmed) || TOOL_BASH_OUTPUT_HEADER_RE.exec(trimmed);
    if (headerMatch) {
      exitCode = normalizeExitCode(headerMatch[1]);
      if (i + 1 < lines.length && lines[i + 1].trim() === 'STDOUT:') {
        atStdout = true;
        i += 1;
      } else {
        plainBuffer.push(line);
        i += 1;
        continue;
      }
    } else if (trimmed === 'STDOUT:') {
      atStdout = true;
    }

    if (!atStdout) {
      plainBuffer.push(line);
      i += 1;
      continue;
    }

    flushText();
    i += 1; // move after STDOUT:

    const stdoutLines: string[] = [];
    const stderrLines: string[] = [];
    let parsingStderr = false;

    while (i < lines.length) {
      const current = lines[i] || '';
      const currentTrimmed = current.trim();
      if (!parsingStderr && currentTrimmed === 'STDERR:') {
        parsingStderr = true;
        i += 1;
        continue;
      }
      // Heuristic boundary for duplicated command output blocks.
      if (currentTrimmed === 'STDOUT:' && (stdoutLines.length > 0 || stderrLines.length > 0)) {
        break;
      }
      if (BASH_OUTPUT_HEADER_RE.test(currentTrimmed) || TOOL_BASH_OUTPUT_HEADER_RE.test(currentTrimmed)) {
        const next = lines[i + 1] || '';
        if (next.trim() === 'STDOUT:') break;
      }
      if (parsingStderr) stderrLines.push(current);
      else stdoutLines.push(current);
      i += 1;
    }

    chunks.push({
      type: 'bash',
      exitCode,
      stdout: trimTrailingEmptyLines(stdoutLines).join('\n'),
      stderr: trimTrailingEmptyLines(stderrLines).join('\n'),
    });
  }

  flushText();
  return chunks.length > 0 ? chunks : [{ type: 'text', text }];
};

const lineCount = (text: string) => {
  if (!text) return 0;
  return text.split('\n').length;
};

const renderAgentMessageBody = (text: string) => {
  const chunks = parseBashOutputChunks(text);
  if (chunks.length === 1 && chunks[0]?.type === 'text') {
    return <MarkdownContent text={chunks[0].text} />;
  }
  return (
    <div className="space-y-2">
      {chunks.map((chunk, idx) => {
        if (chunk.type === 'text') {
          return <MarkdownContent key={`text-${idx}`} text={chunk.text} />;
        }
        const stdoutLines = lineCount(chunk.stdout);
        const stderrLines = lineCount(chunk.stderr);
        return (
          <details
            key={`bash-${idx}`}
            className="rounded-md border border-slate-200 dark:border-white/10 bg-slate-50/80 dark:bg-white/[0.03] text-[11px]"
          >
            <summary className="cursor-pointer px-2 py-1.5 text-slate-600 dark:text-slate-300 select-none flex flex-wrap items-center gap-2">
              <span className="font-semibold">Bash output</span>
              {chunk.exitCode && <span className="font-mono text-[10px]">exit {chunk.exitCode}</span>}
              <span className="text-[10px]">stdout {stdoutLines} line{stdoutLines === 1 ? '' : 's'}</span>
              <span className="text-[10px]">stderr {stderrLines} line{stderrLines === 1 ? '' : 's'}</span>
            </summary>
            <div className="px-2 pb-2 space-y-1.5">
              <div className="rounded border border-slate-200/80 dark:border-white/10 bg-white dark:bg-black/30">
                <div className="px-2 py-1 text-[10px] uppercase tracking-wider text-slate-500 border-b border-slate-200/70 dark:border-white/10">
                  Stdout
                </div>
                <pre className="m-0 max-h-80 overflow-auto custom-scrollbar p-2 font-mono text-[11px] leading-5 whitespace-pre-wrap break-words">
                  {chunk.stdout || '(empty)'}
                </pre>
              </div>
              <div className="rounded border border-slate-200/80 dark:border-white/10 bg-white dark:bg-black/30">
                <div className="px-2 py-1 text-[10px] uppercase tracking-wider text-slate-500 border-b border-slate-200/70 dark:border-white/10">
                  Stderr
                </div>
                <pre className="m-0 max-h-64 overflow-auto custom-scrollbar p-2 font-mono text-[11px] leading-5 whitespace-pre-wrap break-words">
                  {chunk.stderr || '(empty)'}
                </pre>
              </div>
            </div>
          </details>
        );
      })}
    </div>
  );
};

const sanitizeAgentMessageText = (text: string) => {
  if (!text) return '';
  // Optimization: if it's a raw tool result from system, we often want to hide it entirely
  if (text.startsWith('Read:') || text.startsWith('Tool Read:')) return '';
  
  const withoutEmbedded = text.replace(TOOL_JSON_EMBEDDED_RE, '').trim();
  const lines = withoutEmbedded.split('\n');
  const readFileRelated = /read:|content omitted in chat/i.test(withoutEmbedded);
  const kept: string[] = [];
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) {
      if (kept.length > 0 && kept[kept.length - 1] !== '') kept.push('');
      continue;
    }
    if (parseToolNameFromLine(trimmed)) continue;
    if (/^Tool\s+Bash\s*:\s*Bash output \(exit_code:/i.test(trimmed)) {
      kept.push(trimmed.replace(/^Tool\s+Bash\s*:\s*/i, ''));
      continue;
    }
    if (TOOL_RESULT_LINE_RE.test(trimmed)) continue;
    if (START_AUTONOMOUS_LINE_RE.test(trimmed)) continue;
    if (CONTENT_OMITTED_LINE_RE.test(trimmed)) continue;
    if (readFileRelated && looksLikeCodeLine(trimmed)) continue;
    kept.push(line);
  }
  return kept.join('\n').replace(/\n{3,}/g, '\n\n').trim();
};

const contextMessageToChatMessage = (msg: AgentRunContextMessage): ChatMessage => {
  const timestampMs = Number(msg.timestamp || 0) * 1000;
  const role = roleFromSender(msg.from_id);
  const content = role === 'user' ? msg.content : sanitizeAgentMessageText(msg.content);
  return {
    role,
    from: msg.from_id,
    to: msg.to_id || undefined,
    text: content,
    timestamp: timestampMs > 0 ? new Date(timestampMs).toLocaleTimeString() : '',
    timestampMs,
  };
};

const sameMessageIdentity = (a: ChatMessage, b: ChatMessage) => {
  const aFrom = normalizeAgentKey(a.from || a.role);
  const bFrom = normalizeAgentKey(b.from || b.role);
  const aTo = normalizeAgentKey(a.to || '');
  const bTo = normalizeAgentKey(b.to || '');
  if (aFrom !== bFrom || aTo !== bTo) return false;
  const aText = normalizeMessageTextForDedup(a.text);
  const bText = normalizeMessageTextForDedup(b.text);
  if (!hasStrongContentOverlap(aText, bText)) return false;
  const ta = a.timestampMs ?? 0;
  const tb = b.timestampMs ?? 0;
  if (ta <= 0 || tb <= 0) return true;
  return Math.abs(ta - tb) <= 120_000;
};

const mergeMessageStreams = (contextMessages: ChatMessage[], liveMessages: ChatMessage[]) => {
  if (contextMessages.length === 0) return liveMessages;
  if (liveMessages.length === 0) return contextMessages;
  const merged = [...contextMessages];
  for (const live of liveMessages) {
    if (merged.some((contextMsg) => sameMessageIdentity(contextMsg, live))) continue;
    merged.push(live);
  }
  return merged.sort((a, b) => {
    const ta = a.timestampMs ?? 0;
    const tb = b.timestampMs ?? 0;
    if (ta <= 0 && tb <= 0) return 0;
    if (ta <= 0) return 1;
    if (tb <= 0) return -1;
    return ta - tb;
  });
};

const dedupeActivityEntries = (entries: string[]) => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of entries) {
    const clean = String(raw || '').trim();
    if (!clean || seen.has(clean)) continue;
    seen.add(clean);
    out.push(clean);
  }
  if (!out.includes('Model loading...')) return out;
  const rest = out.filter((entry) => entry !== 'Model loading...');
  return ['Model loading...', ...rest];
};

const isProgressLineText = (text?: string) => {
  const t = String(text || '').trim();
  if (!t) return false;
  return (
    t === 'Thinking...' ||
    t === 'Model loading...' ||
    t === 'Reading file...' ||
    t.startsWith('Reading file:') ||
    t === 'Writing file...' ||
    t.startsWith('Writing file:') ||
    t === 'Editing file...' ||
    t.startsWith('Editing file:') ||
    t === 'Running command...' ||
    t.startsWith('Running command:') ||
    t === 'Searching...' ||
    t.startsWith('Searching:') ||
    t === 'Listing files...' ||
    t.startsWith('Listing files:') ||
    t === 'Delegating...' ||
    t.startsWith('Delegating to subagent:') ||
    t === 'Calling tool...' ||
    t.startsWith('Calling tool:')
  );
};

const summarizeCollapsedActivity = (entries: string[], inProgress = false) => {
  const normalized = entries.map((entry) => entry.toLowerCase());
  const readCount = normalized.filter((v) => v.startsWith('read ') || v.includes('reading file')).length;
  const searchCount = normalized.filter((v) => v.startsWith('searched for ') || v.includes('searching') || v.includes('grep')).length;
  const runCount = normalized.filter((v) => v.startsWith('ran command') || v.includes('running command')).length;
  const delegateCount = normalized.filter((v) => v.startsWith('delegated to ') || v.includes('delegating')).length;
  const writeCount = normalized.filter((v) => v.startsWith('wrote ') || v.includes('writing file')).length;
  const editCount = normalized.filter((v) => v.startsWith('edited ') || v.includes('editing file')).length;
  const listCount = normalized.filter((v) => v.startsWith('listed files') || v.includes('listing files') || v.includes('glob')).length;

  if (readCount > 0 || searchCount > 0 || listCount > 0) {
    const parts: string[] = [];
    if (readCount > 0) parts.push(`${readCount} file${readCount > 1 ? 's' : ''}`);
    if (searchCount > 0) parts.push(`${searchCount} search${searchCount > 1 ? 'es' : ''}`);
    if (listCount > 0) parts.push(`${listCount} list${listCount > 1 ? 's' : ''}`);
    return `${inProgress ? 'Exploring' : 'Explored'} ${parts.join(', ')}`;
  }

  const parts: string[] = [];
  if (runCount > 0) parts.push(`${runCount} command${runCount > 1 ? 's' : ''}`);
  if (delegateCount > 0) parts.push(`${delegateCount} delegation${delegateCount > 1 ? 's' : ''}`);
  if (writeCount > 0) parts.push(`${writeCount} file write${writeCount > 1 ? 's' : ''}`);
  if (editCount > 0) parts.push(`${editCount} file edit${editCount > 1 ? 's' : ''}`);
  if (listCount > 0) parts.push(`${listCount} listing${listCount > 1 ? 's' : ''}`);
  if (parts.length > 0) return `Worked: ${parts.join(', ')}`;
  
  const first = entries[0];
  const last = entries[entries.length - 1];
  if (first === last) return last;
  return `${first} -> ${last}`;
};

const activityHeadline = (msg: ChatMessage, entries: string[]) => {
  const computed = summarizeCollapsedActivity(entries, msg.isGenerating);
  const summary = msg.isGenerating ? (computed || msg.activitySummary || '') : (msg.activitySummary || computed || '');
  if (!msg.isGenerating) return summary;
  return summary || entries[entries.length - 1] || '';
};

const activityEntriesForDetails = (msg: ChatMessage, entries: string[]) => {
  if (msg.isGenerating) return entries;
  // Hide transient status-only lines (Thinking, Model loading) once work is done,
  // but keep tool activity lines (even "doing" forms) so the detail section stays expandable.
  return entries.filter((entry) => {
    const t = entry.trim();
    return t !== 'Thinking...' && t !== 'Thinking' && t !== 'Model loading...' && t !== 'Model loading' && t !== 'Running';
  });
};

const activityEntriesForMessage = (msg: ChatMessage): string[] => {
  const entries = Array.isArray(msg.activityEntries) ? msg.activityEntries : [];
  if (entries.length > 0) return dedupeActivityEntries(entries);
  if (isProgressLineText(msg.text)) return dedupeActivityEntries([msg.text]);
  return [];
};

const collapseProgressMessages = (messages: ChatMessage[]): ChatMessage[] => {
  const out: ChatMessage[] = [];
  const pendingByAgent = new Map<string, string[]>();
  const pendingTsByAgent = new Map<string, number>();
  const pendingGeneratingByAgent = new Map<string, boolean>();

  const appendPendingToOutput = (agentId: string, to?: string) => {
    const pending = pendingByAgent.get(agentId);
    if (!pending || pending.length === 0) return;
    const ts = pendingTsByAgent.get(agentId);
    const isGenerating = !!pendingGeneratingByAgent.get(agentId);
    const deduped = dedupeActivityEntries(pending);
    if (deduped.length === 0) {
      pendingByAgent.delete(agentId);
      pendingTsByAgent.delete(agentId);
      pendingGeneratingByAgent.delete(agentId);
      return;
    }
    out.push({
      role: roleFromSender(agentId),
      from: agentId,
      to: to || 'user',
      text: '',
      timestamp: ts ? new Date(ts).toLocaleTimeString() : '',
      timestampMs: ts,
      isGenerating,
      activityEntries: deduped,
      activitySummary: summarizeCollapsedActivity(deduped, isGenerating),
    });
    pendingByAgent.delete(agentId);
    pendingTsByAgent.delete(agentId);
    pendingGeneratingByAgent.delete(agentId);
  };

  for (const msg of messages) {
    if (msg.role === 'user') {
      for (const key of Array.from(pendingByAgent.keys())) {
        appendPendingToOutput(key, msg.to);
      }
      out.push(msg);
      continue;
    }

    const agentId = normalizeAgentKey(msg.from || msg.role);
    const entries = activityEntriesForMessage(msg);
    const body = String(msg.text || '').trim();
    const onlyProgress = !body || isProgressLineText(body);

    if (onlyProgress) {
      if (entries.length > 0) {
        const existing = pendingByAgent.get(agentId) || [];
        pendingByAgent.set(agentId, dedupeActivityEntries([...existing, ...entries]));
        if (!pendingTsByAgent.has(agentId) && msg.timestampMs) {
          pendingTsByAgent.set(agentId, msg.timestampMs);
        }
        if (msg.isGenerating) {
          pendingGeneratingByAgent.set(agentId, true);
        }
      }
      continue;
    }

    if (pendingByAgent.has(agentId)) {
      const isGenerating = !!pendingGeneratingByAgent.get(agentId) || !!msg.isGenerating;
      const merged = dedupeActivityEntries([
        ...(pendingByAgent.get(agentId) || []),
        ...entries,
      ]);
      pendingByAgent.delete(agentId);
      pendingTsByAgent.delete(agentId);
      pendingGeneratingByAgent.delete(agentId);
      out.push({
        ...msg,
        isGenerating,
        activityEntries: merged.length > 0 ? merged : msg.activityEntries,
        activitySummary:
          merged.length > 0
            ? summarizeCollapsedActivity(merged, isGenerating)
            : msg.activitySummary,
      });
      continue;
    }

    out.push({
      ...msg,
      activityEntries: entries.length > 0 ? entries : msg.activityEntries,
      activitySummary:
        entries.length > 0
          ? summarizeCollapsedActivity(entries, !!msg.isGenerating)
          : msg.activitySummary,
    });
  }

  for (const key of Array.from(pendingByAgent.keys())) {
    appendPendingToOutput(key);
  }

  return out;
};

const formatRunLabel = (run: AgentRunInfo) => {
  const ts = Number(run.started_at || 0);
  const time = ts > 0 ? new Date(ts * 1000).toLocaleTimeString() : '-';
  const shortId = run.run_id.length > 10 ? run.run_id.slice(0, 10) : run.run_id;
  return `${run.status} • ${time} • ${shortId}`;
};

type TimelineEvent = {
  ts: number;
  label: string;
  detail?: string;
  kind: 'run' | 'subagent' | 'tool' | 'task';
};

type ToolIntent = {
  name: string;
  detail?: string;
};

const formatTs = (ts?: number) => {
  if (!ts || ts <= 0) return '-';
  return new Date(ts * 1000).toLocaleTimeString();
};

const previewValue = (value: string, maxChars = 100) =>
  value.length <= maxChars ? value : `${value.slice(0, maxChars)}... (${value.length} chars)`;

const parseToolIntent = (content: string): ToolIntent | null => {
  const trimmed = content.trim();
  if (!trimmed) return null;
  if (/^Calling tool:/i.test(trimmed)) {
    const name = trimmed.replace(/^Calling tool:\s*/i, '').trim();
    return { name: name || 'unknown' };
  }
  if (/^Running command:/i.test(trimmed)) {
    const cmd = trimmed.replace(/^Running command:\s*/i, '').trim();
    return { name: 'Bash', detail: cmd || undefined };
  }
  if (/^Delegating to subagent:/i.test(trimmed)) {
    const target = trimmed.replace(/^Delegating to subagent:\s*/i, '').trim();
    return { name: 'delegate_to_agent', detail: target ? `target=${target}` : undefined };
  }
  if (!trimmed.startsWith('{')) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (!parsed || typeof parsed !== 'object') return null;
    const type = typeof parsed.type === 'string' ? parsed.type : '';
    if (type === 'tool') {
      const tool = typeof parsed.tool === 'string' ? parsed.tool : 'tool';
      if (tool === 'Bash' || tool === 'bash') {
        const cmd = typeof parsed.args?.cmd === 'string' ? parsed.args.cmd.trim() : '';
        return { name: tool, detail: cmd ? previewValue(cmd) : undefined };
      }
      if (tool === 'delegate_to_agent') {
        const target = typeof parsed.args?.target_agent_id === 'string'
          ? parsed.args.target_agent_id.trim()
          : '';
        return {
          name: tool,
          detail: target ? `target=${target}` : undefined,
        };
      }
      return { name: tool };
    }
    if (type && type !== 'finalize_task') {
      return { name: type };
    }
  } catch (_e) {
    // ignore non-json
  }
  return null;
};

const parseTaskEvent = (content: string): string | null => {
  const trimmed = content.trim();
  if (!trimmed.startsWith('{')) return null;
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed?.type === 'finalize_task') return 'Finalized task';
  } catch (_e) {
    // ignore non-json
  }
  return null;
};

const hasReadFileActivity = (entries?: string[]) =>
  Array.isArray(entries) &&
  entries.some((entry) => {
    const t = String(entry || '').trim();
    return /^Calling tool:\s*read\b/i.test(t) || /^Reading file(?::|\.\.\.)/i.test(t);
  });

const looksLikeFileDump = (text: string) => {
  const lines = text.split('\n');
  if (lines.length < 40) return false;
  const codeish = lines.filter((line) =>
    /^\s*(\/\/|#include|use\s+\w|import\s+\w|fn\s+\w|class\s+\w|def\s+\w|const\s+\w|let\s+\w|pub\s+\w|[{}();]|<\/?\w+)/.test(
      line
    )
  ).length;
  return codeish >= Math.min(25, Math.floor(lines.length * 0.4));
};

const redactFileDumpForReadFile = (text: string) => {
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

const visibleMessageText = (msg: ChatMessage) => {
  if (msg.role === 'user') return msg.text;
  const sanitized = sanitizeAgentMessageText(msg.text);
  if (!hasReadFileActivity(msg.activityEntries)) return sanitized;
  return redactFileDumpForReadFile(sanitized);
};

const buildRunTimeline = (
  run?: AgentRunInfo,
  messages: AgentRunContextMessage[] = [],
  children: AgentRunInfo[] = []
): TimelineEvent[] => {
  const events: TimelineEvent[] = [];
  if (run) {
    events.push({
      ts: Number(run.started_at || 0),
      label: `Run started (${run.agent_id})`,
      kind: 'run',
    });
    if (run.ended_at) {
      events.push({
        ts: Number(run.ended_at || 0),
        label: `Run ended (${run.status})`,
        detail: run.detail || undefined,
        kind: 'run',
      });
    }
  }
  for (const child of children) {
    events.push({
      ts: Number(child.started_at || 0),
      label: `Spawned subagent: ${child.agent_id}`,
      kind: 'subagent',
    });
    if (child.ended_at) {
      events.push({
        ts: Number(child.ended_at || 0),
        label: `Subagent returned: ${child.agent_id} (${child.status})`,
        detail: child.detail || undefined,
        kind: 'subagent',
      });
    }
  }
  for (const msg of messages) {
    const tool = parseToolIntent(msg.content);
    if (tool) {
      events.push({
        ts: Number(msg.timestamp || 0),
        label: `Tool: ${tool.name}`,
        detail: [tool.detail, `${msg.from_id}${msg.to_id ? ` -> ${msg.to_id}` : ''}`]
          .filter(Boolean)
          .join(' • '),
        kind: 'tool',
      });
      continue;
    }
    const taskEvent = parseTaskEvent(msg.content);
    if (taskEvent) {
      events.push({
        ts: Number(msg.timestamp || 0),
        label: taskEvent,
        detail: msg.from_id,
        kind: 'task',
      });
    }
  }
  return events
    .filter((evt) => evt.ts > 0)
    .sort((a, b) => a.ts - b.ts)
    .slice(-40);
};

export const ChatPanel: React.FC<{
  chatMessages: ChatMessage[];
  queuedMessages: QueuedChatItem[];
  chatEndRef: React.RefObject<HTMLDivElement | null>;
  selectedAgent: string;
  setSelectedAgent: (value: string) => void;
  skills: SkillInfo[];
  agents: AgentInfo[];
  mainAgents: AgentInfo[];
  subagents: SubagentInfo[];
  mainRunIds?: Record<string, string>;
  subagentRunIds?: Record<string, string>;
  runningMainRunIds?: Record<string, string>;
  runningSubagentRunIds?: Record<string, string>;
  mainRunHistory?: Record<string, AgentRunInfo[]>;
  subagentRunHistory?: Record<string, AgentRunInfo[]>;
  cancellingRunIds?: Record<string, boolean>;
  onCancelRun?: (runId: string) => void | Promise<void>;
  onSendMessage: (message: string, targetAgent?: string) => void;
}> = ({
  chatMessages,
  queuedMessages,
  chatEndRef,
  selectedAgent,
  setSelectedAgent,
  skills,
  agents,
  mainAgents,
  subagents,
  mainRunIds,
  subagentRunIds,
  runningMainRunIds,
  runningSubagentRunIds,
  mainRunHistory,
  subagentRunHistory,
  cancellingRunIds,
  onCancelRun,
  onSendMessage,
}) => {
  const [chatInput, setChatInput] = useState('');
  const [showSkillDropdown, setShowSkillDropdown] = useState(false);
  const [skillFilter, setSkillFilter] = useState('');
  const [showAgentDropdown, setShowAgentDropdown] = useState(false);
  const [agentFilter, setAgentFilter] = useState('');
  const [selectedSuggestionIndex, setSelectedSuggestionIndex] = useState(0);
  const [openSubagentId, setOpenSubagentId] = useState<string | null>(null);
  const [selectedMainRunByAgent, setSelectedMainRunByAgent] = useState<Record<string, string>>({});
  const [selectedSubagentRunById, setSelectedSubagentRunById] = useState<Record<string, string>>({});
  const [pinnedMainRunByAgent, setPinnedMainRunByAgent] = useState<Record<string, boolean>>({});
  const [pinnedSubagentRunById, setPinnedSubagentRunById] = useState<Record<string, boolean>>({});
  const [mainMessageFilter, setMainMessageFilter] = useState('');
  const [subagentMessageFilter, setSubagentMessageFilter] = useState('');
  const [runContextById, setRunContextById] = useState<Record<string, AgentRunContextResponse>>({});
  const [loadingContextByRunId, setLoadingContextByRunId] = useState<Record<string, boolean>>({});
  const [contextErrorByRunId, setContextErrorByRunId] = useState<Record<string, string>>({});
  const [childrenByRunId, setChildrenByRunId] = useState<Record<string, AgentRunInfo[]>>({});
  const [loadingChildrenByRunId, setLoadingChildrenByRunId] = useState<Record<string, boolean>>({});
  const [childrenErrorByRunId, setChildrenErrorByRunId] = useState<Record<string, string>>({});
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  const mainAgentIds = useMemo(
    () => mainAgents.map((agent) => normalizeAgentKey(agent.name)),
    [mainAgents]
  );

  const visibleMessages = useMemo(() => {
    const selected = normalizeAgentKey(selectedAgent);
    const filtered = chatMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      if (msg.role === 'user') {
        return !to || to === selected;
      }
      if (from === selected || to === selected) return true;
      if (from === 'user') return to === selected;
      return false;
    });
    return sortMessagesByTime(filtered);
  }, [chatMessages, selectedAgent]);

  const visibleQueued = useMemo(
    () => queuedMessages.filter((item) => normalizeAgentKey(item.agent_id) === normalizeAgentKey(selectedAgent)),
    [queuedMessages, selectedAgent]
  );

  const selectedSubagent = useMemo(
    () => subagents.find((sub) => sub.id === openSubagentId) || null,
    [subagents, openSubagentId]
  );
  const selectedAgentKey = normalizeAgentKey(selectedAgent);
  const selectedMainRunOptions = useMemo(
    () => mainRunHistory?.[selectedAgentKey] || [],
    [mainRunHistory, selectedAgentKey]
  );
  const selectedMainRunOverride = selectedMainRunByAgent[selectedAgentKey];
  const selectedMainPinned = !!pinnedMainRunByAgent[selectedAgentKey];
  const selectedMainRunId =
    selectedMainPinned &&
    selectedMainRunOverride &&
    selectedMainRunOptions.some((run) => run.run_id === selectedMainRunOverride)
      ? selectedMainRunOverride
      : mainRunIds?.[selectedAgentKey] || selectedMainRunOptions[0]?.run_id;
  const selectedMainRunningRunId = runningMainRunIds?.[selectedAgentKey];
  const selectedSubagentKey = selectedSubagent ? normalizeAgentKey(selectedSubagent.id) : '';
  const selectedSubagentRunOptions = useMemo(
    () => (selectedSubagent ? subagentRunHistory?.[selectedSubagentKey] || [] : []),
    [selectedSubagent, subagentRunHistory, selectedSubagentKey]
  );
  const selectedSubagentRunOverride = selectedSubagentKey
    ? selectedSubagentRunById[selectedSubagentKey]
    : undefined;
  const selectedSubagentPinned = selectedSubagentKey
    ? !!pinnedSubagentRunById[selectedSubagentKey]
    : false;
  const selectedSubagentRunId =
    selectedSubagent &&
    selectedSubagentPinned &&
    selectedSubagentRunOverride &&
    selectedSubagentRunOptions.some((run) => run.run_id === selectedSubagentRunOverride)
      ? selectedSubagentRunOverride
      : selectedSubagent
        ? subagentRunIds?.[selectedSubagentKey] || selectedSubagentRunOptions[0]?.run_id
        : undefined;
  const selectedSubagentRunningRunId = selectedSubagent
    ? runningSubagentRunIds?.[selectedSubagentKey]
    : undefined;
  const selectedMainContext = selectedMainRunId ? runContextById[selectedMainRunId] : undefined;
  const selectedSubagentContext = selectedSubagentRunId ? runContextById[selectedSubagentRunId] : undefined;
  const selectedMainContextError = selectedMainRunId ? contextErrorByRunId[selectedMainRunId] : undefined;
  const selectedSubagentContextError = selectedSubagentRunId
    ? contextErrorByRunId[selectedSubagentRunId]
    : undefined;
  const selectedMainContextLoading = selectedMainRunId
    ? !!loadingContextByRunId[selectedMainRunId]
    : false;
  const selectedSubagentContextLoading = selectedSubagentRunId
    ? !!loadingContextByRunId[selectedSubagentRunId]
    : false;
  const selectedMainChildren = useMemo(
    () => (selectedMainRunId ? childrenByRunId[selectedMainRunId] || [] : []),
    [selectedMainRunId, childrenByRunId]
  );
  const selectedSubagentChildren = useMemo(
    () => (selectedSubagentRunId ? childrenByRunId[selectedSubagentRunId] || [] : []),
    [selectedSubagentRunId, childrenByRunId]
  );
  const selectedMainChildrenLoading = selectedMainRunId
    ? !!loadingChildrenByRunId[selectedMainRunId]
    : false;
  const selectedSubagentChildrenLoading = selectedSubagentRunId
    ? !!loadingChildrenByRunId[selectedSubagentRunId]
    : false;
  const selectedMainChildrenError = selectedMainRunId
    ? childrenErrorByRunId[selectedMainRunId]
    : undefined;
  const selectedSubagentChildrenError = selectedSubagentRunId
    ? childrenErrorByRunId[selectedSubagentRunId]
    : undefined;
  const subagentMessages = useMemo(() => {
    if (!selectedSubagent) return [];
    const id = normalizeAgentKey(selectedSubagent.id);
    const filtered = chatMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      return from === id || to === id;
    });
    return sortMessagesByTime(filtered);
  }, [chatMessages, selectedSubagent]);
  const mainContextMessages = useMemo(
    () => (selectedMainContext?.messages || []).map(contextMessageToChatMessage),
    [selectedMainContext]
  );
  const selectedSubagentContextMessages = useMemo(
    () => (selectedSubagentContext?.messages || []).map(contextMessageToChatMessage),
    [selectedSubagentContext]
  );
  const displayedMainMessages = useMemo(
    () => collapseProgressMessages(mergeMessageStreams(mainContextMessages, visibleMessages)),
    [mainContextMessages, visibleMessages]
  );
  const displayedSubagentMessages = useMemo(
    () => mergeMessageStreams(selectedSubagentContextMessages, subagentMessages),
    [selectedSubagentContextMessages, subagentMessages]
  );
  const filteredMainMessages = useMemo(() => {
    const q = mainMessageFilter.trim().toLowerCase();
    if (!q) return displayedMainMessages;
    return displayedMainMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      const activitySummary = (msg.activitySummary || '').toLowerCase();
      const activityLines = (msg.activityEntries || []).join('\n').toLowerCase();
      return (
        msg.text.toLowerCase().includes(q) ||
        activitySummary.includes(q) ||
        activityLines.includes(q) ||
        from.includes(q) ||
        to.includes(q)
      );
    });
  }, [displayedMainMessages, mainMessageFilter]);
  const filteredSubagentMessages = useMemo(() => {
    const q = subagentMessageFilter.trim().toLowerCase();
    if (!q) return displayedSubagentMessages;
    return displayedSubagentMessages.filter((msg) => {
      const from = normalizeAgentKey(msg.from || msg.role);
      const to = normalizeAgentKey(msg.to || '');
      return (
        msg.text.toLowerCase().includes(q) ||
        from.includes(q) ||
        to.includes(q)
      );
    });
  }, [displayedSubagentMessages, subagentMessageFilter]);
  const selectedMainTimeline = useMemo(
    () => buildRunTimeline(selectedMainContext?.run, selectedMainContext?.messages || [], selectedMainChildren),
    [selectedMainContext, selectedMainChildren]
  );
  const selectedSubagentTimeline = useMemo(
    () => buildRunTimeline(selectedSubagentContext?.run, selectedSubagentContext?.messages || [], selectedSubagentChildren),
    [selectedSubagentContext, selectedSubagentChildren]
  );

  const fetchRunContext = useCallback(
    (runId?: string, force = false) => {
      if (!runId) return;
      if (loadingContextByRunId[runId]) return;
      if (!force && runContextById[runId]) return;
      setLoadingContextByRunId((prev) => ({ ...prev, [runId]: true }));
      setContextErrorByRunId((prev) => {
        const next = { ...prev };
        delete next[runId];
        return next;
      });
      void (async () => {
        try {
          const url = new URL('/api/agent-context', window.location.origin);
          url.searchParams.append('run_id', runId);
          url.searchParams.append('view', 'raw');
          const resp = await fetch(url.toString());
          if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
          const data = (await resp.json()) as AgentRunContextResponse;
          setRunContextById((prev) => ({ ...prev, [runId]: data }));
        } catch (e) {
          const errorMessage = e instanceof Error ? e.message : String(e);
          setContextErrorByRunId((prev) => ({ ...prev, [runId]: errorMessage }));
        } finally {
          setLoadingContextByRunId((prev) => {
            const next = { ...prev };
            delete next[runId];
            return next;
          });
        }
      })();
    },
    [runContextById, loadingContextByRunId]
  );

  const fetchRunChildren = useCallback(
    (runId?: string, force = false) => {
      if (!runId) return;
      if (loadingChildrenByRunId[runId]) return;
      if (!force && childrenByRunId[runId]) return;
      setLoadingChildrenByRunId((prev) => ({ ...prev, [runId]: true }));
      setChildrenErrorByRunId((prev) => {
        const next = { ...prev };
        delete next[runId];
        return next;
      });
      void (async () => {
        try {
          const url = new URL('/api/agent-children', window.location.origin);
          url.searchParams.append('run_id', runId);
          const resp = await fetch(url.toString());
          if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
          const data = (await resp.json()) as AgentRunInfo[];
          setChildrenByRunId((prev) => ({ ...prev, [runId]: Array.isArray(data) ? data : [] }));
        } catch (e) {
          const errorMessage = e instanceof Error ? e.message : String(e);
          setChildrenErrorByRunId((prev) => ({ ...prev, [runId]: errorMessage }));
        } finally {
          setLoadingChildrenByRunId((prev) => {
            const next = { ...prev };
            delete next[runId];
            return next;
          });
        }
      })();
    },
    [childrenByRunId, loadingChildrenByRunId]
  );

  useEffect(() => {
    if (!openSubagentId) return;
    if (!subagents.some((sub) => sub.id === openSubagentId)) {
      setOpenSubagentId(null);
    }
  }, [openSubagentId, subagents]);

  useEffect(() => {
    if (!selectedMainPinned || !selectedMainRunOverride) return;
    if (selectedMainRunOptions.some((run) => run.run_id === selectedMainRunOverride)) return;
    setPinnedMainRunByAgent((prev) => {
      const next = { ...prev };
      delete next[selectedAgentKey];
      return next;
    });
    setSelectedMainRunByAgent((prev) => {
      const next = { ...prev };
      delete next[selectedAgentKey];
      return next;
    });
  }, [selectedMainPinned, selectedMainRunOverride, selectedMainRunOptions, selectedAgentKey]);

  useEffect(() => {
    if (!selectedSubagentKey || !selectedSubagentPinned || !selectedSubagentRunOverride) return;
    if (selectedSubagentRunOptions.some((run) => run.run_id === selectedSubagentRunOverride)) return;
    setPinnedSubagentRunById((prev) => {
      const next = { ...prev };
      delete next[selectedSubagentKey];
      return next;
    });
    setSelectedSubagentRunById((prev) => {
      const next = { ...prev };
      delete next[selectedSubagentKey];
      return next;
    });
  }, [selectedSubagentKey, selectedSubagentPinned, selectedSubagentRunOverride, selectedSubagentRunOptions]);

  useEffect(() => {
    fetchRunContext(selectedMainRunId);
  }, [selectedMainRunId, fetchRunContext]);

  useEffect(() => {
    fetchRunContext(selectedSubagentRunId);
  }, [selectedSubagentRunId, fetchRunContext]);

  useEffect(() => {
    fetchRunChildren(selectedMainRunId);
  }, [selectedMainRunId, fetchRunChildren]);

  useEffect(() => {
    fetchRunChildren(selectedSubagentRunId);
  }, [selectedSubagentRunId, fetchRunChildren]);

  useEffect(() => {
    if (!selectedMainRunningRunId && !selectedSubagentRunningRunId) return;
    const id = window.setInterval(() => {
      if (selectedMainRunningRunId) fetchRunContext(selectedMainRunningRunId, true);
      if (selectedSubagentRunningRunId) fetchRunContext(selectedSubagentRunningRunId, true);
      if (selectedMainRunningRunId) fetchRunChildren(selectedMainRunningRunId, true);
      if (selectedSubagentRunningRunId) fetchRunChildren(selectedSubagentRunningRunId, true);
    }, 2000);
    return () => window.clearInterval(id);
  }, [selectedMainRunningRunId, selectedSubagentRunningRunId, fetchRunContext, fetchRunChildren]);

  const resizeInput = () => {
    if (!inputRef.current) return;
    inputRef.current.style.height = '0px';
    const next = Math.min(inputRef.current.scrollHeight, 220);
    inputRef.current.style.height = `${next}px`;
  };

  useEffect(() => {
    resizeInput();
  }, [chatInput]);

  const send = () => {
    if (!chatInput.trim()) return;
    const userMessage = chatInput;
    setChatInput('');
    setShowSkillDropdown(false);
    setShowAgentDropdown(false);

    const mentionMatch = userMessage.trim().match(/^@([a-zA-Z0-9_-]+)\b/);
    let mentionAgent: string | undefined;
    if (mentionMatch?.[1]) {
      const mentioned = normalizeAgentKey(mentionMatch[1]);
      if (mainAgentIds.includes(mentioned)) {
        mentionAgent = mentioned;
        setSelectedAgent(mentioned);
      }
    }

    const targetAgent = mentionAgent || selectedAgent;
    onSendMessage(userMessage, targetAgent);
    window.setTimeout(resizeInput, 0);
  };

  const buildSkillSuggestions = () => {
    const suggestions: {
      key: string;
      label: string;
      description?: string;
      apply: () => void;
    }[] = [];

    const beforeSlash = chatInput.substring(0, chatInput.lastIndexOf('/'));

    if ('mode'.includes(skillFilter)) {
      suggestions.push({
        key: 'cmd-mode',
        label: '/mode',
        description: 'Switch between chat and auto.',
        apply: () => {
          setChatInput(`${beforeSlash}/mode `);
          setSkillFilter('mode');
          setShowSkillDropdown(true);
        },
      });
    }

    if (skillFilter.startsWith('mode')) {
      [
        { cmd: '/mode chat', desc: 'Plain-text answers (summaries, explanations).' },
        { cmd: '/mode auto', desc: 'Structured planning responses (user stories + criteria).' },
      ].forEach((item) => {
        suggestions.push({
          key: item.cmd,
          label: item.cmd,
          description: item.desc,
          apply: () => {
            setChatInput(`${item.cmd} `);
            setShowSkillDropdown(false);
          },
        });
      });
    }

    skills
      .filter(
        (skill) =>
          skill.name.toLowerCase().includes(skillFilter) ||
          skill.description.toLowerCase().includes(skillFilter)
      )
      .forEach((skill) => {
        suggestions.push({
          key: `skill-${skill.name}`,
          label: `/${skill.name}`,
          description: skill.description,
          apply: () => {
            setChatInput(`${beforeSlash}/${skill.name} `);
            setShowSkillDropdown(false);
          },
        });
      });

    return suggestions;
  };

  return (
    <section className="h-full flex flex-col bg-white dark:bg-[#0f0f0f] rounded-xl border border-slate-200 dark:border-white/5 overflow-hidden min-h-0 relative">
      <div className="px-1.5 py-1 border-b border-slate-200 dark:border-white/5 bg-slate-50/70 dark:bg-white/[0.02] space-y-1">
        {selectedMainRunId && (
          <details className="rounded-md border border-slate-200 dark:border-white/10 bg-white/80 dark:bg-black/20 px-2 py-1 text-[10px] text-slate-600 dark:text-slate-300">
            <summary className="cursor-pointer flex flex-wrap items-center gap-2">
              <span className="font-semibold uppercase tracking-wider text-slate-500">Run</span>
              <span className="font-mono truncate">{selectedMainRunId}</span>
              {selectedMainContext?.run?.status && (
                <span className={cn('px-1.5 py-0.5 rounded-full uppercase tracking-wide', statusBadgeClass(selectedMainContext.run.status))}>
                  {selectedMainContext.run.status}
                </span>
              )}
            </summary>
            <div className="mt-1.5 flex flex-wrap items-center gap-2">
              {selectedMainRunOptions.length > 1 && (
                <select
                  value={selectedMainRunId}
                  onChange={(e) => {
                    const runId = e.target.value;
                    setSelectedMainRunByAgent((prev) => ({ ...prev, [selectedAgentKey]: runId }));
                    setPinnedMainRunByAgent((prev) => ({ ...prev, [selectedAgentKey]: true }));
                  }}
                  className="text-[10px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none min-w-[10rem]"
                  title="Select run context"
                >
                  {selectedMainRunOptions.map((run) => (
                    <option key={run.run_id} value={run.run_id}>
                      {formatRunLabel(run)}
                    </option>
                  ))}
                </select>
              )}
              <button
                onClick={() => {
                  if (selectedMainPinned) {
                    setPinnedMainRunByAgent((prev) => {
                      const next = { ...prev };
                      delete next[selectedAgentKey];
                      return next;
                    });
                    setSelectedMainRunByAgent((prev) => {
                      const next = { ...prev };
                      delete next[selectedAgentKey];
                      return next;
                    });
                  } else {
                    setSelectedMainRunByAgent((prev) => ({ ...prev, [selectedAgentKey]: selectedMainRunId }));
                    setPinnedMainRunByAgent((prev) => ({ ...prev, [selectedAgentKey]: true }));
                  }
                }}
                className={cn(
                  'px-2 py-1 rounded border text-[10px] font-semibold',
                  selectedMainPinned
                    ? 'bg-slate-100 text-slate-600 border-slate-300'
                    : 'bg-blue-50 text-blue-600 border-blue-200'
                )}
                title={selectedMainPinned ? 'Unpin run selection' : 'Pin this run selection'}
              >
                {selectedMainPinned ? 'Unpin' : 'Pin'}
              </button>
              {selectedMainRunningRunId && onCancelRun && (
                <button
                  onClick={() => onCancelRun(selectedMainRunningRunId)}
                  disabled={!!cancellingRunIds?.[selectedMainRunningRunId]}
                  className={cn(
                    'px-2 py-1 rounded border text-[10px] font-semibold transition-colors',
                    cancellingRunIds?.[selectedMainRunningRunId]
                      ? 'bg-slate-100 text-slate-400 border-slate-200 cursor-not-allowed'
                      : 'bg-red-50 text-red-600 border-red-200 hover:bg-red-100'
                  )}
                  title={selectedMainRunningRunId}
                >
                  {cancellingRunIds?.[selectedMainRunningRunId] ? 'Cancelling...' : 'Cancel Run'}
                </button>
              )}
              {selectedMainContextLoading && <span className="text-blue-500">Loading context...</span>}
              {selectedMainContextError && <span className="text-red-500">Context error: {selectedMainContextError}</span>}
              {selectedMainChildrenLoading && <span className="text-blue-500">Loading child runs...</span>}
              {selectedMainChildrenError && <span className="text-red-500">Children error: {selectedMainChildrenError}</span>}
            </div>
            {selectedMainContext?.summary && (
              <div className="mt-1 text-slate-500 dark:text-slate-400">
                msgs {selectedMainContext.summary.message_count} • user {selectedMainContext.summary.user_messages} • agent {selectedMainContext.summary.agent_messages} • system {selectedMainContext.summary.system_messages}
              </div>
            )}
          </details>
        )}

        {subagents.length > 0 && (
          <details className="rounded-md border border-slate-200 dark:border-white/10 bg-white/80 dark:bg-black/20 px-2 py-1 text-[10px]">
            <summary className="cursor-pointer font-semibold uppercase tracking-wider text-slate-500 dark:text-slate-400 flex items-center gap-1">
              <Sparkles size={11} />
              Subagents ({subagents.length})
            </summary>
            <div className="mt-1 flex flex-wrap items-center gap-1.5">
              {subagents.map((sub) => (
                <button
                  key={sub.id}
                  onClick={() => setOpenSubagentId(sub.id)}
                  className={cn(
                    'px-2 py-1 rounded-md text-[10px] border transition-colors flex items-center gap-1',
                    openSubagentId === sub.id
                      ? 'bg-blue-600 text-white border-blue-600'
                      : 'bg-slate-100 dark:bg-white/5 border-slate-200 dark:border-white/10 hover:bg-slate-200 dark:hover:bg-white/10'
                  )}
                >
                  <span className="font-semibold">{sub.id}</span>
                  <span className={cn('px-1.5 py-0.5 rounded-full uppercase tracking-wide', statusBadgeClass(sub.status))}>
                    {sub.status}
                  </span>
                </button>
              ))}
            </div>
          </details>
        )}
      </div>

      <div className="flex-1 overflow-y-scroll px-2 py-1.5 flex flex-col gap-2 custom-scrollbar min-h-0">
        <div className="flex items-center justify-between gap-2 mb-1">
          {selectedMainTimeline.length > 0 ? (
            <details className="text-[10px] text-slate-500">
              <summary className="cursor-pointer">Timeline ({selectedMainTimeline.length})</summary>
              <div className="mt-1 space-y-1 max-h-28 overflow-auto custom-scrollbar pr-2">
                {selectedMainTimeline.map((evt, idx) => (
                  <div key={`${evt.ts}-${evt.label}-${idx}`} className="text-[10px] text-slate-500 dark:text-slate-400">
                    {formatTs(evt.ts)} • {evt.label}
                    {evt.detail ? ` • ${evt.detail}` : ''}
                  </div>
                ))}
              </div>
            </details>
          ) : (
            <div />
          )}
          <input
            value={mainMessageFilter}
            onChange={(e) => setMainMessageFilter(e.target.value)}
            placeholder="Filter messages"
            className="w-52 text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
          />
        </div>
        {filteredMainMessages.length === 0 && (
          <div className="self-center mt-12 max-w-md text-center">
            <div className="text-sm font-semibold text-slate-600 dark:text-slate-300">
              No messages for {selectedAgent}
            </div>
            <div className="mt-2 text-xs text-slate-500">
              Send a message to this main agent or switch tabs.
            </div>
          </div>
        )}
        {filteredMainMessages.map((msg, i) => {
          const key = `${msg.timestamp}-${i}-${msg.from || msg.role}-${msg.text.slice(0, 24)}`;
          const isUser = msg.role === 'user';
          const displayText = visibleMessageText(msg);
          const activityEntries = activityEntriesForMessage(msg);
          const hasActivity = !isUser && activityEntries.length > 0;
          const detailActivityEntries = hasActivity ? activityEntriesForDetails(msg, activityEntries) : [];
          const hasActivityDetails = !isUser && detailActivityEntries.length > 0;
          const hasActivitySummary = !isUser && (hasActivity || !!msg.activitySummary);
          const isStatusLine = isProgressLineText(msg.text);
          const activitySummaryText = hasActivitySummary
            ? (hasActivity ? summarizeCollapsedActivity(activityEntries, !!msg.isGenerating) : (msg.activitySummary || ''))
            : '';
          const hideStatusBodyText = hasActivitySummary && (
            isStatusLine ||
            displayText.trim().length === 0 ||
            displayText.trim() === activitySummaryText
          );
          const messageClass = isUser
            ? 'bg-slate-100 dark:bg-white/10 text-slate-900 dark:text-slate-100 rounded-md px-2.5 py-1.5'
            : msg.isThinking
              ? 'text-slate-500 dark:text-slate-400 italic opacity-60'
              : isStatusLine && !hasActivity
                ? 'text-blue-700 dark:text-blue-300 italic'
                : 'text-slate-800 dark:text-slate-200';
          return (
          <div
            key={key}
            className={cn('w-full flex', isUser ? 'justify-end' : 'justify-start')}
          >
            <div
              className={cn(
                'max-w-[96%] text-[13px] leading-relaxed',
                messageClass
              )}
            >
              {hasActivitySummary && (
                hasActivityDetails ? (
                  <details className="group mb-1 text-[11px] text-slate-500 dark:text-slate-400" open={msg.isGenerating || undefined}>
                    <summary className="cursor-pointer select-none list-none flex items-center gap-1.5 [&::-webkit-details-marker]:hidden">
                      {msg.isGenerating ? (
                        <span className="flex gap-0.5">
                          <span className="w-1 h-1 bg-blue-500 rounded-full animate-bounce [animation-delay:-0.3s]" />
                          <span className="w-1 h-1 bg-blue-500 rounded-full animate-bounce [animation-delay:-0.15s]" />
                          <span className="w-1 h-1 bg-blue-500 rounded-full animate-bounce" />
                        </span>
                      ) : (
                        <span className="text-[9px] text-slate-400 dark:text-slate-500 transition-transform group-open:rotate-90">&#9654;</span>
                      )}
                      <span>{activityHeadline(msg, activityEntries)}</span>
                    </summary>
                    <div className="mt-1 space-y-0.5 pl-4 border-l-2 border-slate-200/80 dark:border-white/10">
                      {detailActivityEntries.map((entry, idx) => (
                        <div key={`${idx}-${entry}`} className="truncate text-slate-400 dark:text-slate-500">
                          {entry}
                        </div>
                      ))}
                    </div>
                  </details>
                ) : (
                  <div className="mb-1 text-[11px] text-slate-500 dark:text-slate-400">
                    {activityHeadline(msg, activityEntries)}
                  </div>
                )
              )}
              {(() => {
                if (isUser || (isStatusLine && !hasActivity)) return displayText;
                if (hideStatusBodyText) return null;
                try {
                  const parsed = JSON.parse(displayText);
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
                              <div key={idx} className="text-[11px] opacity-90">
                                - {story}
                              </div>
                            ))}
                          </div>
                        )}
                        {criteria.length > 0 && (
                          <div className="space-y-1 text-[11px]">
                            <div className="uppercase tracking-wider text-[9px] text-slate-500">Acceptance Criteria</div>
                            {criteria.map((crit: string, idx: number) => (
                              <div key={idx} className="text-[11px] opacity-90">
                                - {crit}
                              </div>
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
                      }))
                      .filter((item: any) => item.path);
                    const truncatedCount = Number(parsed.truncated_count || 0);
                    const reviewHint =
                      typeof parsed.review_hint === 'string' ? parsed.review_hint : '';
                    return (
                      <div className="space-y-2">
                        <div className="font-bold text-blue-500">
                          Changed files ({files.length}
                          {truncatedCount > 0 ? ` +${truncatedCount} more` : ''})
                        </div>
                        {files.map((file: any, idx: number) => (
                          <div
                            key={`${file.path}-${idx}`}
                            className="flex flex-wrap items-center gap-2 rounded-md border border-slate-200 dark:border-white/10 bg-slate-50/80 dark:bg-white/[0.03] px-2 py-1.5 text-[11px]"
                          >
                            <span className="text-slate-500 dark:text-slate-300">
                              {file.summary || 'Updated'}
                            </span>
                            <span className="font-mono text-[11px]">{file.path}</span>
                          </div>
                        ))}
                        {reviewHint && (
                          <div className="text-[11px] text-slate-500 dark:text-slate-400">
                            {reviewHint}
                          </div>
                        )}
                      </div>
                    );
                  }
                  return renderAgentMessageBody(displayText);
                } catch (_e) {
                  return renderAgentMessageBody(displayText);
                }
              })()}
              {msg.isGenerating && <span className="inline-block w-1.5 h-3.5 bg-blue-500 ml-1 animate-pulse align-middle" />}
            </div>
          </div>
        )})}
        <div ref={chatEndRef} />
      </div>

      <div className="sticky bottom-0 z-10 p-2 border-t border-slate-200 dark:border-white/5 space-y-2 bg-slate-50 dark:bg-white/[0.02]">
        {visibleQueued.length > 0 && (
          <div className="rounded-md border border-amber-300/50 bg-amber-50 dark:bg-amber-500/10 px-2 py-1.5 text-[10px] text-amber-800 dark:text-amber-200">
            <div className="font-semibold">Queued messages ({visibleQueued.length})</div>
            <div className="mt-1 space-y-1">
              {visibleQueued.map((item) => (
                <div key={item.id} className="truncate">
                  [{item.agent_id}] {item.preview}
                </div>
              ))}
            </div>
          </div>
        )}
        <div className="flex gap-2 bg-white dark:bg-black/20 p-1.5 rounded-xl border border-slate-300/80 dark:border-white/10 relative items-end">
          {showSkillDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-52 overflow-y-auto z-[70]">
              <div className="px-3 py-2 text-[10px] text-slate-500 border-b border-slate-200 dark:border-white/10">
                Type to filter skills • Press Enter to send
              </div>
              {(() => {
                const suggestions = buildSkillSuggestions();
                return suggestions.map((item, idx) => (
                  <button
                    key={item.key}
                    onClick={item.apply}
                    className={cn(
                      'w-full px-3 py-2 text-left text-xs border-b border-slate-200 dark:border-white/5 last:border-none',
                      idx === selectedSuggestionIndex
                        ? 'bg-blue-500/10 text-blue-600'
                        : 'hover:bg-slate-100 dark:hover:bg-white/5'
                    )}
                  >
                    <div className="font-bold text-blue-500">{item.label}</div>
                    {item.description && <div className="text-slate-500 text-[10px]">{item.description}</div>}
                  </button>
                ));
              })()}
              {buildSkillSuggestions().length === 0 && (
                <div className="p-3 text-[10px] text-slate-500 italic">No matching skills found</div>
              )}
            </div>
          )}
          {showAgentDropdown && (
            <div className="absolute bottom-full left-0 right-0 mb-2 bg-white dark:bg-[#141414] border border-slate-200 dark:border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto z-[70]">
              {agents
                .filter((agent) => mainAgentIds.includes(normalizeAgentKey(agent.name)))
                .filter((agent) => agent.name.toLowerCase().includes(agentFilter))
                .map((agent) => (
                  <button
                    key={agent.name}
                    onClick={() => {
                      const beforeAt = chatInput.substring(0, chatInput.lastIndexOf('@'));
                      const label = agent.name.charAt(0).toUpperCase() + agent.name.slice(1);
                      setChatInput(`${beforeAt}@${label} `);
                      setShowAgentDropdown(false);
                      setSelectedAgent(agent.name.toLowerCase());
                    }}
                    className="w-full px-3 py-2 text-left hover:bg-slate-100 dark:hover:bg-white/5 text-xs border-b border-slate-200 dark:border-white/5 last:border-none"
                  >
                    <div className="font-bold text-purple-500">@{agent.name.charAt(0).toUpperCase() + agent.name.slice(1)}</div>
                    <div className="text-slate-500 text-[10px]">{agent.description}</div>
                  </button>
                ))}
            </div>
          )}
          <textarea
            ref={inputRef}
            value={chatInput}
            onChange={(e) => {
              const val = e.target.value;
              setChatInput(val);
              if (val.includes('/') && !val.includes(' ', val.lastIndexOf('/'))) {
                setSkillFilter(val.substring(val.lastIndexOf('/') + 1).toLowerCase());
                setShowSkillDropdown(true);
                setShowAgentDropdown(false);
                setSelectedSuggestionIndex(0);
              } else if (val.includes('@') && !val.includes(' ', val.lastIndexOf('@'))) {
                setAgentFilter(val.substring(val.lastIndexOf('@') + 1).toLowerCase());
                setShowAgentDropdown(true);
                setShowSkillDropdown(false);
              } else {
                setShowSkillDropdown(false);
                setShowAgentDropdown(false);
              }
            }}
            onKeyDown={(e) => {
              const suggestions = showSkillDropdown ? buildSkillSuggestions() : [];
              if (showSkillDropdown && suggestions.length > 0 && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
                e.preventDefault();
                const delta = e.key === 'ArrowDown' ? 1 : -1;
                setSelectedSuggestionIndex((prev) => (prev + delta + suggestions.length) % suggestions.length);
                return;
              }
              if (showSkillDropdown && suggestions.length > 0 && e.key === 'Enter') {
                e.preventDefault();
                suggestions[selectedSuggestionIndex]?.apply();
                return;
              }
              if (e.key === 'Enter' && !e.shiftKey && !showSkillDropdown && !showAgentDropdown) {
                e.preventDefault();
                send();
              }
              if (e.key === 'Escape') {
                setShowSkillDropdown(false);
                setShowAgentDropdown(false);
              }
            }}
            placeholder="Message...  (/ for skills, @ for agents, Shift+Enter for newline)"
            rows={1}
            className="flex-1 bg-transparent border-none px-1.5 py-1.5 text-[13px] outline-none resize-none min-h-[34px] max-h-[200px] leading-5"
          />
          <button
            onClick={send}
            className="w-8 h-8 rounded-lg bg-blue-600 text-white flex items-center justify-center hover:bg-blue-500 transition-colors"
            title="Send"
          >
            <Send size={14} />
          </button>
        </div>
      </div>

      {selectedSubagent && (
        <div className="absolute inset-y-0 right-0 w-[min(26rem,95%)] bg-white dark:bg-[#0f0f0f] border-l border-slate-200 dark:border-white/10 shadow-2xl z-[65] flex flex-col">
          <div className="px-4 py-3 border-b border-slate-200 dark:border-white/10 flex items-start justify-between gap-3">
            <div>
              <div className="text-xs font-bold uppercase tracking-wider text-slate-500">Subagent Context</div>
              <div className="mt-1 text-sm font-semibold text-slate-900 dark:text-slate-100">{selectedSubagent.id}</div>
              <div className="mt-1 text-[10px] text-slate-500 dark:text-slate-400">
                {selectedSubagent.folder}/{selectedSubagent.file}
              </div>
            </div>
            <div className="flex items-center gap-2">
              <span className={cn('text-[10px] px-2 py-1 rounded-full uppercase tracking-wide', statusBadgeClass(selectedSubagent.status))}>
                {selectedSubagent.status}
              </span>
              {selectedSubagentRunningRunId && onCancelRun && (
                <button
                  onClick={() => onCancelRun(selectedSubagentRunningRunId)}
                  disabled={!!cancellingRunIds?.[selectedSubagentRunningRunId]}
                  className={cn(
                    'px-2 py-1 rounded-lg text-[10px] font-semibold border transition-colors',
                    cancellingRunIds?.[selectedSubagentRunningRunId]
                      ? 'bg-slate-100 text-slate-400 border-slate-200 cursor-not-allowed'
                      : 'bg-red-50 text-red-600 border-red-200 hover:bg-red-100'
                  )}
                  title={selectedSubagentRunningRunId}
                >
                  {cancellingRunIds?.[selectedSubagentRunningRunId] ? 'Cancelling...' : 'Cancel Run'}
                </button>
              )}
              <button
                onClick={() => setOpenSubagentId(null)}
                className="p-1.5 rounded-lg hover:bg-slate-100 dark:hover:bg-white/10 text-slate-500"
                title="Close"
              >
                <X size={14} />
              </button>
            </div>
          </div>

          <div className="p-4 border-b border-slate-200 dark:border-white/10">
            <div className="text-[10px] uppercase tracking-widest text-slate-500 mb-2">Active Paths</div>
            <div className="space-y-1 max-h-28 overflow-auto custom-scrollbar">
              {selectedSubagent.paths.map((path) => (
                <div key={path} className="text-[11px] font-mono text-slate-600 dark:text-slate-300 truncate">
                  {path}
                </div>
              ))}
            </div>
          </div>

          <div className="flex-1 overflow-auto p-4 space-y-3 custom-scrollbar">
            <div className="text-[10px] uppercase tracking-widest text-slate-500">Messages</div>
            {selectedSubagentRunId && (
              <div className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/5 px-2.5 py-2 text-[10px] text-slate-500 dark:text-slate-400">
                <div className="font-mono text-slate-600 dark:text-slate-300 break-all">{selectedSubagentRunId}</div>
                {selectedSubagentRunOptions.length > 1 && (
                  <div className="mt-1">
                    <select
                      value={selectedSubagentRunId}
                      onChange={(e) => {
                        const runId = e.target.value;
                        if (!selectedSubagentKey) return;
                        setSelectedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: runId }));
                        setPinnedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: true }));
                      }}
                      className="w-full text-[10px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
                      title="Select subagent run context"
                    >
                      {selectedSubagentRunOptions.map((run) => (
                        <option key={run.run_id} value={run.run_id}>
                          {formatRunLabel(run)}
                        </option>
                      ))}
                    </select>
                  </div>
                )}
                {selectedSubagentKey && selectedSubagentRunId && (
                  <div className="mt-1">
                    <button
                      onClick={() => {
                        if (selectedSubagentPinned) {
                          setPinnedSubagentRunById((prev) => {
                            const next = { ...prev };
                            delete next[selectedSubagentKey];
                            return next;
                          });
                          setSelectedSubagentRunById((prev) => {
                            const next = { ...prev };
                            delete next[selectedSubagentKey];
                            return next;
                          });
                        } else {
                          setSelectedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: selectedSubagentRunId }));
                          setPinnedSubagentRunById((prev) => ({ ...prev, [selectedSubagentKey]: true }));
                        }
                      }}
                      className={cn(
                        'px-2 py-1 rounded border text-[10px] font-semibold',
                        selectedSubagentPinned
                          ? 'bg-slate-100 text-slate-600 border-slate-300'
                          : 'bg-blue-50 text-blue-600 border-blue-200'
                      )}
                    >
                      {selectedSubagentPinned ? 'Unpin' : 'Pin'}
                    </button>
                  </div>
                )}
                {selectedSubagentContext?.summary && (
                  <div className="mt-1">
                    messages: {selectedSubagentContext.summary.message_count} • user: {selectedSubagentContext.summary.user_messages} • agent: {selectedSubagentContext.summary.agent_messages} • system: {selectedSubagentContext.summary.system_messages}
                  </div>
                )}
                {selectedSubagentContextLoading && <div className="mt-1 text-blue-500">Loading context...</div>}
                {selectedSubagentContextError && <div className="mt-1 text-red-500">Context error: {selectedSubagentContextError}</div>}
                {selectedSubagentChildrenLoading && <div className="mt-1 text-blue-500">Loading child runs...</div>}
                {selectedSubagentChildrenError && <div className="mt-1 text-red-500">Children error: {selectedSubagentChildrenError}</div>}
              </div>
            )}
            <div className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50/70 dark:bg-white/[0.03] px-2.5 py-2 space-y-2">
              <div className="flex items-center justify-between gap-2">
                <div className="text-[10px] uppercase tracking-widest text-slate-500">Context Tools</div>
                <input
                  value={subagentMessageFilter}
                  onChange={(e) => setSubagentMessageFilter(e.target.value)}
                  placeholder="Filter messages"
                  className="w-40 text-[11px] bg-white dark:bg-black/30 border border-slate-200 dark:border-white/10 rounded px-2 py-1 outline-none"
                />
              </div>
              {selectedSubagentTimeline.length > 0 && (
                <details>
                  <summary className="cursor-pointer text-[11px] font-semibold text-slate-600 dark:text-slate-300">
                    Timeline ({selectedSubagentTimeline.length})
                  </summary>
                  <div className="mt-1.5 space-y-1.5 max-h-28 overflow-auto custom-scrollbar">
                    {selectedSubagentTimeline.map((evt, idx) => (
                      <div key={`${evt.ts}-${evt.label}-${idx}`} className="text-[11px] text-slate-600 dark:text-slate-300">
                        <span className="font-mono text-[10px] text-slate-500 mr-2">{formatTs(evt.ts)}</span>
                        <span className="font-semibold">{evt.label}</span>
                        {evt.detail && <span className="text-slate-500"> • {evt.detail}</span>}
                      </div>
                    ))}
                  </div>
                </details>
              )}
            </div>
            {filteredSubagentMessages.length === 0 && (
              <div className="text-xs italic text-slate-500">No context messages captured for this subagent yet.</div>
            )}
            {filteredSubagentMessages.slice(-20).map((msg, idx) => (
              <div key={`${msg.timestamp}-${idx}-${msg.from || msg.role}`} className="rounded-lg border border-slate-200 dark:border-white/10 bg-slate-50 dark:bg-white/5 p-2.5">
                <div className="text-[9px] uppercase tracking-wider text-slate-500 mb-1">
                  {(msg.from || msg.role).toUpperCase()} {msg.to ? `→ ${msg.to.toUpperCase()}` : ''}
                </div>
                <div className="text-[12px] text-slate-700 dark:text-slate-200 break-words">
                  {renderAgentMessageBody(visibleMessageText(msg))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  );
};
