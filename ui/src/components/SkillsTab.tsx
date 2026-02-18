import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronDown, ChevronRight, Download, FilePlus2, Pencil, Save, Search, Store, Trash2, Wrench, X } from 'lucide-react';
import type { MarketplaceSkill, SkillInfoFull, SkillFileInfo } from '../types';
import { CM6Editor } from './CM6Editor';

const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm';

const sourceBadgeCls: Record<string, string> = {
  Embedded: 'bg-slate-500/10 text-slate-600 dark:text-slate-400',
  Global: 'bg-purple-500/10 text-purple-600 dark:text-purple-400',
  Project: 'bg-green-500/10 text-green-600 dark:text-green-400',
};

const defaultSkillTemplate = (name: string) => `---
name: ${name}
description: ${name} skill.
tools: []
---

Skill instructions go here.
`;

interface SkillGroup {
  label: string;
  source: string;
  skills: SkillInfoFull[];
}

type SubTab = 'local' | 'marketplace';

export const SkillsTab: React.FC<{
  projectRoot: string;
}> = ({ projectRoot }) => {
  const [subTab, setSubTab] = useState<SubTab>('local');
  const [allSkills, setAllSkills] = useState<SkillInfoFull[]>([]);
  const [skillFiles, setSkillFiles] = useState<SkillFileInfo[]>([]);
  const [expandedSkills, setExpandedSkills] = useState<Set<string>>(new Set());
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [editingSkill, setEditingSkill] = useState<string | null>(null);
  const [editContent, setEditContent] = useState<string>('');
  const [savedEditContent, setSavedEditContent] = useState<string>('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Marketplace state
  const [mpQuery, setMpQuery] = useState('');
  const [mpResults, setMpResults] = useState<MarketplaceSkill[]>([]);
  const [mpLoading, setMpLoading] = useState(false);
  const [mpInstalling, setMpInstalling] = useState<Set<string>>(new Set());
  const [mpUninstalling, setMpUninstalling] = useState<Set<string>>(new Set());
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fetchSkills = useCallback(async () => {
    try {
      const resp = await fetch('/api/skills');
      if (resp.ok) setAllSkills(await resp.json());
    } catch { /* ignore */ }
  }, []);

  const fetchSkillFiles = useCallback(async () => {
    if (!projectRoot) return;
    try {
      const resp = await fetch(`/api/skill-files?project_root=${encodeURIComponent(projectRoot)}`);
      if (resp.ok) setSkillFiles(await resp.json());
    } catch { /* ignore */ }
  }, [projectRoot]);

  useEffect(() => {
    fetchSkills();
    fetchSkillFiles();
  }, [fetchSkills, fetchSkillFiles]);

  // Marketplace: load popular on tab open
  useEffect(() => {
    if (subTab === 'marketplace' && mpResults.length === 0 && !mpQuery) {
      fetchMarketplaceList();
    }
  }, [subTab]);

  const fetchMarketplaceList = async () => {
    setMpLoading(true);
    try {
      const resp = await fetch('/api/marketplace/list?limit=20');
      if (resp.ok) setMpResults(await resp.json());
    } catch { /* ignore */ }
    setMpLoading(false);
  };

  const searchMarketplace = async (q: string) => {
    if (!q.trim()) {
      fetchMarketplaceList();
      return;
    }
    setMpLoading(true);
    try {
      const resp = await fetch(`/api/marketplace/search?q=${encodeURIComponent(q)}`);
      if (resp.ok) setMpResults(await resp.json());
    } catch { /* ignore */ }
    setMpLoading(false);
  };

  const handleMpQueryChange = (val: string) => {
    setMpQuery(val);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => searchMarketplace(val), 400);
  };

  const installedNames = new Set(allSkills.map((s) => s.name));

  const installMarketplaceSkill = async (skill: MarketplaceSkill, scope: 'project' | 'global' = 'project') => {
    setMpInstalling((prev) => new Set(prev).add(skill.name));
    setError(null);
    try {
      const resp = await fetch('/api/marketplace/install', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: skill.name,
          repo_url: skill.url || undefined,
          git_ref: skill.git_ref || undefined,
          scope,
          project_root: scope === 'project' ? projectRoot : undefined,
          force: false,
        }),
      });
      if (!resp.ok) {
        setError(await resp.text());
      } else {
        await Promise.all([fetchSkills(), fetchSkillFiles()]);
      }
    } catch (e) {
      setError(String(e));
    }
    setMpInstalling((prev) => {
      const next = new Set(prev);
      next.delete(skill.name);
      return next;
    });
  };

  const uninstallMarketplaceSkill = async (name: string, scope: 'project' | 'global' = 'project') => {
    setMpUninstalling((prev) => new Set(prev).add(name));
    setError(null);
    try {
      const resp = await fetch('/api/marketplace/uninstall', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name,
          scope,
          project_root: scope === 'project' ? projectRoot : undefined,
        }),
      });
      if (!resp.ok) {
        setError(await resp.text());
      } else {
        await Promise.all([fetchSkills(), fetchSkillFiles()]);
      }
    } catch (e) {
      setError(String(e));
    }
    setMpUninstalling((prev) => {
      const next = new Set(prev);
      next.delete(name);
      return next;
    });
  };

  const groups: SkillGroup[] = (() => {
    const bySource: Record<string, SkillInfoFull[]> = { Embedded: [], Global: [], Project: [] };
    for (const skill of allSkills) {
      const src = skill.source?.type || 'Embedded';
      (bySource[src] ??= []).push(skill);
    }
    return [
      { label: 'Embedded', source: 'Embedded', skills: bySource.Embedded },
      { label: 'Global', source: 'Global', skills: bySource.Global },
      { label: 'Project', source: 'Project', skills: bySource.Project },
    ].filter((g) => g.skills.length > 0);
  })();

  const toggleExpanded = (name: string) => {
    setExpandedSkills((prev) => {
      const next = new Set(prev);
      next.has(name) ? next.delete(name) : next.add(name);
      return next;
    });
  };

  const toggleGroup = (source: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      next.has(source) ? next.delete(source) : next.add(source);
      return next;
    });
  };

  const startEditing = (skill: SkillInfoFull) => {
    const file = skillFiles.find((f) => f.name === skill.name);
    if (!file) return;
    loadSkillFile(file.path);
  };

  const loadSkillFile = async (path: string) => {
    try {
      const resp = await fetch(
        `/api/skill-file?project_root=${encodeURIComponent(projectRoot)}&path=${encodeURIComponent(path)}`
      );
      if (!resp.ok) return;
      const data = await resp.json();
      setEditContent(data.content || '');
      setSavedEditContent(data.content || '');
      setEditingSkill(path);
      setError(null);
    } catch { /* ignore */ }
  };

  const saveSkillFile = async () => {
    if (!editingSkill) return;
    setSaving(true);
    setError(null);
    try {
      const resp = await fetch('/api/skill-file', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: projectRoot, path: editingSkill, content: editContent }),
      });
      if (!resp.ok) {
        setError(await resp.text());
        return;
      }
      setSavedEditContent(editContent);
      await Promise.all([fetchSkills(), fetchSkillFiles()]);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const createSkillFile = async () => {
    const raw = prompt('New skill filename (example: my-skill.md):', 'new-skill.md');
    if (!raw) return;
    const filename = raw.trim();
    if (!filename) return;
    const name = filename.replace(/\.md$/i, '');
    const template = defaultSkillTemplate(name);
    try {
      const resp = await fetch('/api/skill-file', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: projectRoot, path: filename, content: template }),
      });
      if (!resp.ok) {
        setError(await resp.text());
        return;
      }
      await Promise.all([fetchSkills(), fetchSkillFiles()]);
      const data = await resp.json();
      if (data.path) loadSkillFile(data.path);
    } catch (e) {
      setError(String(e));
    }
  };

  const deleteSkillFile = async (path: string) => {
    if (!confirm(`Delete skill file ${path}?`)) return;
    try {
      const resp = await fetch('/api/skill-file', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ project_root: projectRoot, path }),
      });
      if (!resp.ok) {
        setError(await resp.text());
        return;
      }
      if (editingSkill === path) {
        setEditingSkill(null);
        setEditContent('');
        setSavedEditContent('');
      }
      await Promise.all([fetchSkills(), fetchSkillFiles()]);
    } catch (e) {
      setError(String(e));
    }
  };

  const isProjectSkill = (skill: SkillInfoFull) => skill.source?.type === 'Project';
  const editDirty = editContent !== savedEditContent;

  // Editor mode
  if (editingSkill) {
    return (
      <div className="flex flex-col h-full min-h-0">
        <div className="px-4 py-2 border-b border-slate-200 dark:border-white/10 flex items-center justify-between bg-slate-50/50 dark:bg-white/[0.02]">
          <div className="flex items-center gap-2">
            <button
              onClick={() => {
                if (editDirty && !confirm('Discard unsaved changes?')) return;
                setEditingSkill(null);
              }}
              className="p-1 rounded hover:bg-slate-200 dark:hover:bg-white/10"
            >
              <X size={14} />
            </button>
            <span className="text-xs font-mono text-slate-600 dark:text-slate-300">{editingSkill}</span>
            {editDirty && <span className="text-[11px] text-amber-600">Unsaved</span>}
          </div>
          <div className="flex items-center gap-1.5">
            {error && <span className="text-[10px] text-red-500 max-w-60 truncate">{error}</span>}
            <button
              onClick={saveSkillFile}
              disabled={saving || !editDirty}
              className="px-2 py-1 rounded text-xs border border-slate-200 dark:border-white/10 hover:bg-slate-50 dark:hover:bg-white/5 disabled:opacity-50"
            >
              <span className="inline-flex items-center gap-1"><Save size={12} /> Save</span>
            </button>
          </div>
        </div>
        <div className="flex-1 min-h-0">
          <CM6Editor value={editContent} onChange={setEditContent} />
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Sub-tab toggle */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1 bg-slate-100 dark:bg-white/5 rounded-lg p-0.5">
          <button
            onClick={() => setSubTab('local')}
            className={`px-3 py-1.5 text-[11px] font-semibold rounded-md transition-colors ${
              subTab === 'local'
                ? 'bg-white dark:bg-white/10 text-slate-900 dark:text-slate-100 shadow-sm'
                : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300'
            }`}
          >
            Local Skills
          </button>
          <button
            onClick={() => setSubTab('marketplace')}
            className={`px-3 py-1.5 text-[11px] font-semibold rounded-md transition-colors inline-flex items-center gap-1 ${
              subTab === 'marketplace'
                ? 'bg-white dark:bg-white/10 text-slate-900 dark:text-slate-100 shadow-sm'
                : 'text-slate-500 hover:text-slate-700 dark:hover:text-slate-300'
            }`}
          >
            <Store size={12} /> Marketplace
          </button>
        </div>
        {subTab === 'local' && (
          <button onClick={createSkillFile} className="flex items-center gap-1 text-[10px] font-bold text-blue-600 hover:text-blue-700">
            <FilePlus2 size={12} /> New Project Skill
          </button>
        )}
      </div>

      {error && (
        <div className="text-xs text-red-500 bg-red-50 dark:bg-red-500/10 rounded-lg px-3 py-2 flex items-center justify-between">
          <span>{error}</span>
          <button onClick={() => setError(null)} className="ml-2 text-red-400 hover:text-red-600"><X size={12} /></button>
        </div>
      )}

      {/* Local Skills tab */}
      {subTab === 'local' && (
        <>
          {groups.map((group) => {
            const collapsed = collapsedGroups.has(group.source);
            return (
              <div key={group.source} className={sectionCls}>
                <button
                  onClick={() => toggleGroup(group.source)}
                  className="w-full flex items-center gap-2 px-4 py-3 text-left"
                >
                  {collapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
                  <span className={`text-[10px] font-bold px-2 py-0.5 rounded-full ${sourceBadgeCls[group.source] || ''}`}>
                    {group.label}
                  </span>
                  <span className="text-[10px] text-slate-500">{group.skills.length} skill{group.skills.length !== 1 ? 's' : ''}</span>
                </button>

                {!collapsed && (
                  <div className="px-4 pb-3 space-y-2">
                    {group.skills.map((skill) => {
                      const expanded = expandedSkills.has(skill.name);
                      const isProject = isProjectSkill(skill);
                      const file = skillFiles.find((f) => f.name === skill.name);
                      return (
                        <div key={skill.name} className="bg-slate-50 dark:bg-white/[0.02] rounded-lg border border-slate-100 dark:border-white/5">
                          <div
                            className="flex items-center gap-2 px-3 py-2 cursor-pointer"
                            onClick={() => toggleExpanded(skill.name)}
                          >
                            {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                            <span className="text-xs font-semibold flex-1">{skill.name}</span>
                            {skill.tool_defs && skill.tool_defs.length > 0 && (
                              <span className="inline-flex items-center gap-1 text-[10px] text-slate-500 bg-slate-200/60 dark:bg-white/5 px-1.5 py-0.5 rounded-full">
                                <Wrench size={10} /> {skill.tool_defs.length}
                              </span>
                            )}
                            {isProject && file && (
                              <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
                                <button
                                  onClick={() => startEditing(skill)}
                                  className="p-1 text-slate-400 hover:text-blue-500 transition-colors"
                                  title="Edit"
                                >
                                  <Pencil size={11} />
                                </button>
                                <button
                                  onClick={() => file && deleteSkillFile(file.path)}
                                  className="p-1 text-slate-400 hover:text-red-500 transition-colors"
                                  title="Delete"
                                >
                                  <Trash2 size={11} />
                                </button>
                              </div>
                            )}
                          </div>

                          {expanded && (
                            <div className="px-3 pb-3 space-y-2">
                              <p className="text-[11px] text-slate-500">{skill.description}</p>

                              {skill.tool_defs && skill.tool_defs.length > 0 && (
                                <div className="overflow-x-auto">
                                  <table className="w-full text-[10px]">
                                    <thead>
                                      <tr className="text-left text-slate-500 border-b border-slate-200 dark:border-white/5">
                                        <th className="py-1 pr-2 font-bold">Tool</th>
                                        <th className="py-1 pr-2 font-bold">Description</th>
                                        <th className="py-1 pr-2 font-bold">Args</th>
                                        <th className="py-1 pr-2 font-bold">Cmd</th>
                                        <th className="py-1 font-bold">Timeout</th>
                                      </tr>
                                    </thead>
                                    <tbody>
                                      {skill.tool_defs.map((tool) => (
                                        <tr key={tool.name} className="border-b border-slate-100 dark:border-white/[0.03]">
                                          <td className="py-1 pr-2 font-mono font-semibold">{tool.name}</td>
                                          <td className="py-1 pr-2 text-slate-500 max-w-40 truncate">{tool.description}</td>
                                          <td className="py-1 pr-2 font-mono">
                                            {Object.keys(tool.args || {}).join(', ') || '-'}
                                          </td>
                                          <td className="py-1 pr-2 font-mono text-slate-500 max-w-40 truncate">{tool.cmd}</td>
                                          <td className="py-1">{tool.timeout_ms}ms</td>
                                        </tr>
                                      ))}
                                    </tbody>
                                  </table>
                                </div>
                              )}

                              {skill.content && (
                                <details className="text-[11px]">
                                  <summary className="text-slate-500 cursor-pointer hover:text-slate-700 dark:hover:text-slate-300">
                                    Content preview
                                  </summary>
                                  <pre className="mt-1 p-2 bg-slate-100 dark:bg-black/20 rounded text-[10px] overflow-x-auto max-h-40 whitespace-pre-wrap">
                                    {skill.content.slice(0, 500)}{skill.content.length > 500 ? '...' : ''}
                                  </pre>
                                </details>
                              )}
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}

          {groups.length === 0 && (
            <div className={`${sectionCls} p-6 text-center`}>
              <p className="text-xs text-slate-400">No skills found. Create a project skill to get started.</p>
            </div>
          )}
        </>
      )}

      {/* Marketplace tab */}
      {subTab === 'marketplace' && (
        <div className="space-y-3">
          {/* Search bar */}
          <div className={`${sectionCls} px-3 py-2`}>
            <div className="flex items-center gap-2">
              <Search size={14} className="text-slate-400 shrink-0" />
              <input
                type="text"
                value={mpQuery}
                onChange={(e) => handleMpQueryChange(e.target.value)}
                placeholder="Search skills..."
                className="flex-1 text-xs bg-transparent outline-none placeholder:text-slate-400"
                autoFocus
              />
              {mpLoading && (
                <span className="text-[10px] text-slate-400 animate-pulse">Loading...</span>
              )}
            </div>
          </div>

          {/* Results */}
          {mpResults.length > 0 ? (
            <div className="space-y-2">
              {mpResults.map((skill) => {
                const isInstalled = installedNames.has(skill.name);
                const isInstalling = mpInstalling.has(skill.name);
                const isUninstalling = mpUninstalling.has(skill.name);

                return (
                  <div key={skill.skill_id || skill.name} className={`${sectionCls} px-4 py-3`}>
                    <div className="flex items-start justify-between gap-3">
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-xs font-semibold">{skill.name}</span>
                          {skill.install_count > 0 && (
                            <span className="text-[10px] text-slate-400 bg-slate-100 dark:bg-white/5 px-1.5 py-0.5 rounded-full">
                              <Download size={9} className="inline mr-0.5 -mt-px" />
                              {skill.install_count}
                            </span>
                          )}
                          {isInstalled && (
                            <span className="text-[10px] font-bold text-green-600 bg-green-500/10 px-1.5 py-0.5 rounded-full">
                              Installed
                            </span>
                          )}
                        </div>
                        {skill.description && (
                          <p className="text-[11px] text-slate-500 mt-0.5 line-clamp-2">{skill.description}</p>
                        )}
                        {skill.url && (
                          <p className="text-[10px] text-slate-400 mt-0.5 truncate font-mono">{skill.url}</p>
                        )}
                      </div>
                      <div className="flex items-center gap-1 shrink-0">
                        {isInstalled ? (
                          <button
                            onClick={() => uninstallMarketplaceSkill(skill.name)}
                            disabled={isUninstalling}
                            className="px-2 py-1 text-[10px] font-semibold rounded-md border border-red-200 dark:border-red-500/20 text-red-600 hover:bg-red-50 dark:hover:bg-red-500/10 disabled:opacity-50"
                          >
                            {isUninstalling ? 'Removing...' : 'Uninstall'}
                          </button>
                        ) : (
                          <button
                            onClick={() => installMarketplaceSkill(skill)}
                            disabled={isInstalling}
                            className="px-2 py-1 text-[10px] font-semibold rounded-md bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
                          >
                            {isInstalling ? 'Installing...' : 'Install'}
                          </button>
                        )}
                      </div>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : (
            !mpLoading && (
              <div className={`${sectionCls} p-6 text-center`}>
                <Store size={24} className="mx-auto text-slate-300 dark:text-slate-600 mb-2" />
                <p className="text-xs text-slate-400">
                  {mpQuery ? 'No skills found for this search.' : 'Search for skills or browse popular ones.'}
                </p>
              </div>
            )
          )}
        </div>
      )}
    </div>
  );
};
