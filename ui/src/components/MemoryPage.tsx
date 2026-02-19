import React, { useState, useEffect, useCallback } from 'react';
import { ArrowLeft, Database, Search, Plus, RefreshCw, Trash2, FolderOpen, CheckCircle, XCircle, Loader2, HardDrive } from 'lucide-react';
import type { MemorySource, MemoryIndexingJob, MemorySearchResult, MemoryServerStatus } from '../types';

const MEMORY_API = '/api/memory';

async function fetchMemoryStatus(): Promise<MemoryServerStatus> {
  const resp = await fetch(`${MEMORY_API}/status`);
  if (!resp.ok) throw new Error(`Memory server returned ${resp.status}`);
  return resp.json();
}

async function fetchSources(): Promise<MemorySource[]> {
  const resp = await fetch(`${MEMORY_API}/resources`);
  if (!resp.ok) throw new Error(`Failed to list sources: ${resp.status}`);
  const data = await resp.json();
  return data.resources || [];
}

async function createSource(name: string, path: string): Promise<MemorySource> {
  const resp = await fetch(`${MEMORY_API}/resources`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, resource_type: 'local', path, include_patterns: [], exclude_patterns: [] }),
  });
  if (!resp.ok) throw new Error(await resp.text());
  return resp.json();
}

async function indexSource(sourceId: string, mode: string): Promise<{ job_id: string }> {
  const resp = await fetch(`${MEMORY_API}/index_source`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ source_id: sourceId, mode }),
  });
  if (!resp.ok) throw new Error(await resp.text());
  return resp.json();
}

async function fetchJobs(): Promise<MemoryIndexingJob[]> {
  const resp = await fetch(`${MEMORY_API}/jobs`);
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.jobs || [];
}

async function searchMemory(query: string, limit = 10): Promise<MemorySearchResult[]> {
  const resp = await fetch(`${MEMORY_API}/search`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ query, limit }),
  });
  if (!resp.ok) return [];
  const data = await resp.json();
  return data.results || [];
}

// --- Sub-components ---

const StatusBadge: React.FC<{ status: string }> = ({ status }) => {
  const s = status.toLowerCase();
  if (s === 'ready') return <span className="inline-flex items-center gap-1 text-[10px] font-bold uppercase tracking-wide text-green-600 dark:text-green-400"><CheckCircle size={12} /> Ready</span>;
  if (s === 'initializing') return <span className="inline-flex items-center gap-1 text-[10px] font-bold uppercase tracking-wide text-amber-600 dark:text-amber-400"><Loader2 size={12} className="animate-spin" /> Initializing</span>;
  if (s === 'error') return <span className="inline-flex items-center gap-1 text-[10px] font-bold uppercase tracking-wide text-red-600 dark:text-red-400"><XCircle size={12} /> Error</span>;
  return <span className="text-[10px] font-bold uppercase tracking-wide text-slate-500">{status}</span>;
};

const JobStatusBadge: React.FC<{ status: string }> = ({ status }) => {
  const s = status.toLowerCase();
  const cls = s === 'completed' ? 'bg-green-500/10 text-green-700 dark:text-green-300'
    : s === 'running' ? 'bg-blue-500/10 text-blue-700 dark:text-blue-300'
    : s === 'failed' ? 'bg-red-500/10 text-red-700 dark:text-red-300'
    : 'bg-slate-500/10 text-slate-600 dark:text-slate-300';
  return <span className={`px-1.5 py-0.5 rounded text-[10px] font-bold uppercase ${cls}`}>{status}</span>;
};

const SourceCard: React.FC<{
  source: MemorySource;
  onReindex: (id: string) => void;
  indexing: boolean;
}> = ({ source, onReindex, indexing }) => {
  const stats = source.stats;
  const job = source.latest_job;
  return (
    <div className="border border-slate-200 dark:border-white/10 rounded-lg p-4 bg-white dark:bg-white/[0.02] hover:bg-slate-50 dark:hover:bg-white/[0.04] transition-colors">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 mb-1">
            <FolderOpen size={14} className="text-blue-500 shrink-0" />
            <span className="font-semibold text-sm truncate">{source.name}</span>
          </div>
          <p className="text-[11px] text-slate-500 dark:text-slate-400 truncate" title={source.path}>{source.path}</p>
        </div>
        <button
          onClick={() => onReindex(source.id)}
          disabled={indexing}
          className="p-1.5 rounded-md text-slate-400 hover:text-blue-500 hover:bg-blue-500/10 transition-colors disabled:opacity-40 shrink-0"
          title="Re-index"
        >
          <RefreshCw size={14} className={indexing ? 'animate-spin' : ''} />
        </button>
      </div>
      <div className="flex items-center gap-4 mt-3 text-[11px] text-slate-500 dark:text-slate-400">
        {stats && (
          <>
            <span>{stats.file_count.toLocaleString()} files</span>
            <span>{stats.chunk_count.toLocaleString()} chunks</span>
            {stats.total_size_bytes > 0 && <span>{(stats.total_size_bytes / 1024 / 1024).toFixed(1)} MB</span>}
          </>
        )}
        {!stats && <span className="italic">Not indexed yet</span>}
        {job && <JobStatusBadge status={job.status} />}
      </div>
    </div>
  );
};

const SearchPanel: React.FC = () => {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<MemorySearchResult[]>([]);
  const [searching, setSearching] = useState(false);

  const doSearch = useCallback(async () => {
    if (!query.trim()) return;
    setSearching(true);
    try {
      const res = await searchMemory(query.trim());
      setResults(res);
    } finally {
      setSearching(false);
    }
  }, [query]);

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <div className="relative flex-1">
          <Search size={14} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-slate-400" />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && doSearch()}
            placeholder="Semantic search across indexed sources..."
            className="w-full pl-8 pr-3 py-2 text-sm bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg outline-none focus:ring-2 focus:ring-blue-500/30"
          />
        </div>
        <button
          onClick={doSearch}
          disabled={searching || !query.trim()}
          className="px-3 py-2 text-sm font-medium bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-40 transition-colors"
        >
          {searching ? <Loader2 size={14} className="animate-spin" /> : 'Search'}
        </button>
      </div>
      {results.length > 0 && (
        <div className="space-y-2 max-h-[60vh] overflow-y-auto">
          {results.map((r, i) => (
            <div key={r.id || i} className="border border-slate-200 dark:border-white/10 rounded-lg p-3 bg-white dark:bg-white/[0.02]">
              <div className="flex items-center gap-2 mb-1.5 text-[11px] text-slate-500 dark:text-slate-400">
                <span className="font-medium text-slate-700 dark:text-slate-300">{r.source_name}</span>
                <span>-</span>
                <span className="truncate">{r.file_path}</span>
                <span className="ml-auto shrink-0 text-[10px] font-mono">{(r.score * 100).toFixed(0)}%</span>
              </div>
              <pre className="text-[12px] text-slate-700 dark:text-slate-300 whitespace-pre-wrap font-mono bg-slate-50 dark:bg-white/[0.03] rounded p-2 max-h-32 overflow-y-auto">{r.content}</pre>
            </div>
          ))}
        </div>
      )}
      {results.length === 0 && query.trim() && !searching && (
        <p className="text-sm text-slate-400 text-center py-4">No results found</p>
      )}
    </div>
  );
};

// --- Main page ---

type MemoryTab = 'sources' | 'search' | 'jobs';

export const MemoryPage: React.FC<{
  onBack: () => void;
}> = ({ onBack }) => {
  const [tab, setTab] = useState<MemoryTab>('sources');
  const [status, setStatus] = useState<MemoryServerStatus | null>(null);
  const [sources, setSources] = useState<MemorySource[]>([]);
  const [jobs, setJobs] = useState<MemoryIndexingJob[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [indexingIds, setIndexingIds] = useState<Set<string>>(new Set());
  const [showAddSource, setShowAddSource] = useState(false);
  const [newSourcePath, setNewSourcePath] = useState('');
  const [newSourceName, setNewSourceName] = useState('');

  const refreshData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [st, sr, jb] = await Promise.all([
        fetchMemoryStatus(),
        fetchSources(),
        fetchJobs(),
      ]);
      setStatus(st);
      setSources(sr);
      setJobs(jb);
    } catch (e: any) {
      setError(e.message || 'Failed to connect to memory server');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refreshData();
    const interval = setInterval(refreshData, 5000);
    return () => clearInterval(interval);
  }, [refreshData]);

  const handleReindex = async (sourceId: string) => {
    setIndexingIds((prev) => new Set(prev).add(sourceId));
    try {
      await indexSource(sourceId, 'auto');
      setTimeout(refreshData, 1000);
    } finally {
      setIndexingIds((prev) => {
        const next = new Set(prev);
        next.delete(sourceId);
        return next;
      });
    }
  };

  const handleAddSource = async () => {
    if (!newSourcePath.trim()) return;
    const name = newSourceName.trim() || newSourcePath.split('/').filter(Boolean).pop() || 'Unnamed';
    try {
      const source = await createSource(name, newSourcePath.trim());
      await indexSource(source.id, 'full');
      setShowAddSource(false);
      setNewSourcePath('');
      setNewSourceName('');
      setTimeout(refreshData, 1000);
    } catch (e: any) {
      alert(e.message || 'Failed to add source');
    }
  };

  const tabs: { id: MemoryTab; label: string }[] = [
    { id: 'sources', label: 'Indexed Projects' },
    { id: 'search', label: 'Search' },
    { id: 'jobs', label: 'Jobs' },
  ];

  return (
    <div className="flex flex-col h-screen bg-slate-100/70 dark:bg-[#0a0a0a] text-slate-900 dark:text-slate-200">
      {/* Header */}
      <header className="flex items-center gap-4 px-6 py-3 border-b border-slate-200 dark:border-white/5 bg-white/90 dark:bg-[#0f0f0f]/90 backdrop-blur-md">
        <button onClick={onBack} className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 transition-colors">
          <ArrowLeft size={16} />
        </button>
        <div className="flex items-center gap-2">
          <Database size={18} className="text-blue-500" />
          <h1 className="text-lg font-bold tracking-tight">Memory</h1>
        </div>
        <div className="ml-4 flex items-center gap-2">
          {status && <StatusBadge status={status.status} />}
          {error && <span className="text-[10px] text-red-500">{error}</span>}
        </div>
        <div className="ml-auto flex items-center gap-1">
          <button onClick={refreshData} className="p-1.5 rounded-md hover:bg-slate-100 dark:hover:bg-white/5 text-slate-500 transition-colors" title="Refresh">
            <RefreshCw size={14} className={loading ? 'animate-spin' : ''} />
          </button>
        </div>
      </header>

      {/* Tab bar */}
      <div className="flex items-center gap-1 px-6 py-2 border-b border-slate-200 dark:border-white/5 bg-white/50 dark:bg-white/[0.02]">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={`px-3 py-1.5 rounded-md text-xs font-semibold transition-colors ${
              tab === t.id
                ? 'bg-blue-600 text-white'
                : 'text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-white/5'
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6">
        {error && !status && (
          <div className="text-center py-20">
            <HardDrive size={40} className="mx-auto text-slate-300 dark:text-slate-600 mb-4" />
            <p className="text-lg font-semibold text-slate-600 dark:text-slate-400 mb-2">Memory Server Unreachable</p>
            <p className="text-sm text-slate-500 dark:text-slate-500 mb-4">Start the memory server with <code className="bg-slate-200 dark:bg-white/10 px-1.5 py-0.5 rounded text-[12px]">ling start</code></p>
          </div>
        )}

        {!error && tab === 'sources' && (
          <div className="max-w-3xl mx-auto space-y-4">
            <div className="flex items-center justify-between">
              <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300">Indexed Projects ({sources.length})</h2>
              <button
                onClick={() => setShowAddSource(true)}
                className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors"
              >
                <Plus size={12} /> Add Source
              </button>
            </div>

            {showAddSource && (
              <div className="border border-blue-500/30 rounded-lg p-4 bg-blue-50/50 dark:bg-blue-500/5 space-y-3">
                <input
                  type="text"
                  value={newSourcePath}
                  onChange={(e) => setNewSourcePath(e.target.value)}
                  placeholder="Directory path (e.g., /Users/you/project)"
                  className="w-full px-3 py-2 text-sm bg-white dark:bg-black/20 border border-slate-200 dark:border-white/10 rounded-lg outline-none focus:ring-2 focus:ring-blue-500/30"
                  autoFocus
                />
                <input
                  type="text"
                  value={newSourceName}
                  onChange={(e) => setNewSourceName(e.target.value)}
                  placeholder="Source name (optional, derived from path)"
                  className="w-full px-3 py-2 text-sm bg-white dark:bg-black/20 border border-slate-200 dark:border-white/10 rounded-lg outline-none focus:ring-2 focus:ring-blue-500/30"
                />
                <div className="flex justify-end gap-2">
                  <button onClick={() => setShowAddSource(false)} className="px-3 py-1.5 text-xs font-medium text-slate-600 dark:text-slate-300 rounded-lg hover:bg-slate-100 dark:hover:bg-white/5">Cancel</button>
                  <button onClick={handleAddSource} disabled={!newSourcePath.trim()} className="px-3 py-1.5 text-xs font-semibold bg-blue-600 text-white rounded-lg hover:bg-blue-700 disabled:opacity-40">Add & Index</button>
                </div>
              </div>
            )}

            {sources.length === 0 && !loading && (
              <div className="text-center py-16">
                <FolderOpen size={32} className="mx-auto text-slate-300 dark:text-slate-600 mb-3" />
                <p className="text-sm text-slate-500">No indexed projects yet</p>
                <p className="text-[11px] text-slate-400 mt-1">Add a directory to start indexing</p>
              </div>
            )}

            <div className="space-y-2">
              {sources.map((s) => (
                <SourceCard
                  key={s.id}
                  source={s}
                  onReindex={handleReindex}
                  indexing={indexingIds.has(s.id)}
                />
              ))}
            </div>
          </div>
        )}

        {!error && tab === 'search' && (
          <div className="max-w-3xl mx-auto">
            <SearchPanel />
          </div>
        )}

        {!error && tab === 'jobs' && (
          <div className="max-w-3xl mx-auto space-y-2">
            <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-300 mb-3">Recent Jobs ({jobs.length})</h2>
            {jobs.length === 0 && (
              <p className="text-sm text-slate-400 text-center py-8">No indexing jobs yet</p>
            )}
            {jobs.map((job) => (
              <div key={job.id} className="border border-slate-200 dark:border-white/10 rounded-lg p-3 bg-white dark:bg-white/[0.02] flex items-center gap-4">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium truncate">{job.source_name}</span>
                    <JobStatusBadge status={job.status} />
                  </div>
                  <div className="flex items-center gap-3 mt-1 text-[11px] text-slate-500 dark:text-slate-400">
                    {job.files_indexed != null && <span>{job.files_indexed} files</span>}
                    {job.chunks_created != null && <span>{job.chunks_created} chunks</span>}
                    <span>{new Date(job.started_at).toLocaleString()}</span>
                  </div>
                  {job.error && <p className="text-[11px] text-red-500 mt-1">{job.error}</p>}
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
