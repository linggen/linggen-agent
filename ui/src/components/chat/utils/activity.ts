export const doingFormOf = (done: string): string | null => {
  if (done.startsWith('Read file:')) return 'Reading file:' + done.slice('Read file:'.length);
  if (done.startsWith('Listed files:')) return 'Listing files:' + done.slice('Listed files:'.length);
  if (done.startsWith('Wrote file:')) return 'Writing file:' + done.slice('Wrote file:'.length);
  if (done.startsWith('Edited file:')) return 'Editing file:' + done.slice('Edited file:'.length);
  if (done.startsWith('Ran command:')) return 'Running command:' + done.slice('Ran command:'.length);
  if (done.startsWith('Searched for:')) return 'Searching:' + done.slice('Searched for:'.length);
  if (done.startsWith('Searched:')) return 'Searching:' + done.slice('Searched:'.length);
  if (done.startsWith('Delegated to ')) return 'Delegating to subagent: ' + done.slice('Delegated to '.length);
  return null;
};

export const dedupeActivityEntries = (entries: string[]) => {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of entries) {
    const clean = String(raw || '').trim();
    if (!clean || seen.has(clean)) continue;
    seen.add(clean);
    out.push(clean);
  }
  const doingToRemove = new Set<string>();
  for (const entry of out) {
    const doing = doingFormOf(entry);
    if (doing && seen.has(doing)) {
      doingToRemove.add(doing);
    }
  }
  const filtered = doingToRemove.size > 0 ? out.filter((e) => !doingToRemove.has(e)) : out;
  if (!filtered.includes('Model loading...')) return filtered;
  const rest = filtered.filter((entry) => entry !== 'Model loading...');
  return ['Model loading...', ...rest];
};

export const isProgressLineText = (text?: string) => {
  const t = String(text || '').trim();
  if (!t) return false;
  return (
    t === 'Thinking...' ||
    t === 'Thinking' ||
    t === 'Model loading...' ||
    t === 'Model loading' ||
    t === 'Running' ||
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
    t.startsWith('Calling tool:') ||
    t.startsWith('Used tool:')
  );
};

export const summarizeCollapsedActivity = (entries: string[], inProgress = false) => {
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

export const formatCompactTokens = (n: number): string => {
  if (!Number.isFinite(n) || n <= 0) return '';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}m`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${(n / 1000).toFixed(1)}k`;
  return `${n}`;
};
