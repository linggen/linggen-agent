/**
 * Agent tree / workspace derivation utilities.
 * Extracted from messageUtils.ts.
 */
import type { AgentTreeItem, AgentWorkInfo, SubagentInfo } from '../types';

type ActivityEntry = {
  path: string;
  agent: string;
  status: string;
  lastModified: number;
};

const collectActivityEntries = (
  nodes: Record<string, AgentTreeItem> | undefined,
  out: ActivityEntry[]
) => {
  if (!nodes) return;
  Object.values(nodes).forEach((item) => {
    if (item.type === 'file') {
      if (!item.path || !item.agent) return;
      out.push({
        path: item.path,
        agent: item.agent,
        status: item.status || 'idle',
        lastModified: Number(item.last_modified || 0),
      });
      return;
    }
    collectActivityEntries(item.children, out);
  });
};

const splitFilePath = (path: string) => {
  const idx = path.lastIndexOf('/');
  if (idx < 0) return { folder: '.', file: path };
  return { folder: path.slice(0, idx) || '.', file: path.slice(idx + 1) };
};

export const buildAgentWorkInfo = (tree: Record<string, AgentTreeItem>): Record<string, AgentWorkInfo> => {
  const entries: ActivityEntry[] = [];
  collectActivityEntries(tree, entries);

  const byAgent = entries.reduce<Record<string, ActivityEntry[]>>((acc, entry) => {
    if (!acc[entry.agent]) acc[entry.agent] = [];
    acc[entry.agent].push(entry);
    return acc;
  }, {});

  const out: Record<string, AgentWorkInfo> = {};
  Object.entries(byAgent).forEach(([agent, list]) => {
    const active = list
      .filter((entry) => entry.status === 'working')
      .sort((a, b) => b.lastModified - a.lastModified);
    const current = active[0];
    if (!current) return;
    const parts = splitFilePath(current.path);
    out[agent] = {
      path: current.path,
      folder: parts.folder,
      file: parts.file,
      status: current.status,
      activeCount: active.length,
    };
  });

  return out;
};

export const normalizeAgentStatus = (
  status?: string
): 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working' => {
  if (status === 'calling_tool') return 'calling_tool';
  if (status === 'model_loading') return 'model_loading';
  if (status === 'thinking') return 'thinking';
  if (status === 'working') return 'working';
  return 'idle';
};

export const buildSubagentInfos = (
  tree: Record<string, AgentTreeItem>,
  mainAgentIds: string[],
): SubagentInfo[] => {
  const entries: ActivityEntry[] = [];
  collectActivityEntries(tree, entries);
  const mainSet = new Set(mainAgentIds.map((id) => id.toLowerCase()));

  const bySubagent = entries
    .filter((entry) => !mainSet.has(entry.agent.toLowerCase()))
    .reduce<Record<string, ActivityEntry[]>>((acc, entry) => {
      if (!acc[entry.agent]) acc[entry.agent] = [];
      acc[entry.agent].push(entry);
      return acc;
    }, {});

  const out: SubagentInfo[] = Object.entries(bySubagent)
    .reduce<SubagentInfo[]>((acc, [id, list]) => {
      const sorted = list.slice().sort((a, b) => b.lastModified - a.lastModified);
      const active = sorted.filter((entry) => entry.status === 'working');
      const current = active[0] || sorted[0];
      if (!current) return acc;

      const parts = splitFilePath(current.path);
      const uniquePaths = Array.from(new Set(sorted.map((entry) => entry.path))).slice(0, 8);
      acc.push({
        id,
        status: active.length > 0 ? 'working' : 'idle',
        path: current.path,
        file: parts.file,
        folder: parts.folder,
        activeCount: active.length,
        paths: uniquePaths,
      });

      return acc;
    }, [])
    .sort((a, b) => {
      const score = (status: string) => (status === 'working' ? 2 : status === 'thinking' ? 1 : 0);
      return score(b.status) - score(a.status) || a.id.localeCompare(b.id);
    });

  return out;
};
