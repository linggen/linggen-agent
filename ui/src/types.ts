export interface ChatMessage {
  role: 'user' | 'agent';
  from?: string;
  to?: string;
  text: string;
  timestamp: string;
  timestampMs?: number;
  isGenerating?: boolean;
  isThinking?: boolean;
  activitySummary?: string;
  activityEntries?: string[];
  contextTokens?: number;
  messageCount?: number;
  toolCount?: number;
  durationMs?: number;
}

export interface UiSseMessage {
  id: string;
  seq: number;
  rev: number;
  ts_ms: number;
  kind: 'message' | 'activity' | 'queue' | 'run' | 'token';
  phase?: string;
  text?: string;
  agent_id?: string;
  session_id?: string;
  project_root?: string;
  data?: any;
}

export interface QueuedChatItem {
  id: string;
  agent_id: string;
  session_id: string;
  preview: string;
  timestamp: number;
}

export interface FileEntry {
  name: string;
  isDir: boolean;
  path: string;
}

export interface WorkspaceState {
  active_task: [any, string] | null;
  user_stories: [any, string] | null;
  tasks: [any, string][];
  messages: [any, string][];
}

export interface ProjectInfo {
  path: string;
  name: string;
  added_at: number;
}

export interface AgentTreeItem {
  type: 'file' | 'dir';
  agent?: string;
  status?: string;
  path?: string;
  last_modified?: number;
  children?: Record<string, AgentTreeItem>;
}

export interface AgentWorkInfo {
  path: string;
  file: string;
  folder: string;
  status: string;
  activeCount: number;
}

export interface SubagentInfo {
  id: string;
  status: string;
  path: string;
  file: string;
  folder: string;
  activeCount: number;
  paths: string[];
}

/** @deprecated Use SkillInfoFull instead */
export type SkillInfo = SkillInfoFull;

export interface AgentInfo {
  name: string;
  description: string;
}

export interface AgentFileInfo {
  agent_id: string;
  name: string;
  description: string;
  path: string;
}

export interface ModelInfo {
  id: string;
  provider: string;
  model: string;
  url: string;
}

export interface OllamaPsModel {
  name: string;
  model: string;
  size: number;
  size_vram: number;
  details: {
    parameter_size: string;
    quantization_level: string;
  };
}

export interface OllamaPsResponse {
  models: OllamaPsModel[];
}

export interface AppConfig {
  models: ModelConfigUI[];
  server: { port: number };
  agent: { max_iters: number; write_safety_mode: string; prompt_loop_breaker?: string | null };
  logging: { level?: string | null; directory?: string | null; retention_days?: number | null };
  agents: { id: string; spec_path: string; model?: string | null }[];
}

export interface ModelConfigUI {
  id: string;
  provider: string;
  url: string;
  model: string;
  api_key?: string | null;
  keep_alive?: string | null;
}

export interface SessionInfo {
  id: string;
  repo_path: string;
  title: string;
  created_at: number;
}

export interface AgentRunInfo {
  run_id: string;
  repo_path: string;
  session_id: string;
  agent_id: string;
  agent_kind?: string | null;
  parent_run_id?: string | null;
  status: 'running' | 'completed' | 'failed' | 'cancelled' | string;
  detail?: string | null;
  started_at: number;
  ended_at?: number | null;
}

export interface AgentRunSummary {
  run_id: string;
  status: string;
  started_at: number;
  ended_at?: number | null;
  child_count: number;
  timeline_events: number;
  last_event_at: number;
}

export interface AgentRunContextSummary {
  message_count: number;
  user_messages: number;
  agent_messages: number;
  system_messages: number;
  started_at: number;
  ended_at?: number | null;
}

export interface AgentRunContextMessage {
  repo_path: string;
  session_id: string;
  agent_id: string;
  from_id: string;
  to_id: string;
  content: string;
  timestamp: number;
  is_observation: boolean;
}

export interface AgentRunContextResponse {
  run: AgentRunInfo;
  summary: AgentRunContextSummary;
  messages?: AgentRunContextMessage[] | null;
}

export interface SkillToolParamDef {
  type: string;
  required: boolean;
  default?: any;
  description: string;
}

export interface SkillToolDef {
  name: string;
  description: string;
  cmd: string;
  args: Record<string, SkillToolParamDef>;
  returns?: string | null;
  timeout_ms: number;
}

export interface SkillInfoFull {
  name: string;
  description: string;
  content: string;
  source: { type: 'Global' | 'Project' | 'Compat' };
  tool_defs: SkillToolDef[];
  user_invocable?: boolean;
  allowed_tools?: string[];
  argument_hint?: string | null;
  model?: string | null;
  trigger?: string | null;
  agent?: string | null;
}

export interface SkillFileInfo {
  name: string;
  path: string;
  source: string;
}

export interface MarketplaceSkill {
  skill_id: string;
  name: string;
  url: string;
  description?: string | null;
  install_count: number;
  git_ref?: string | null;
  content?: string | null;
}

export interface BuiltInSkillInfo {
  name: string;
  description: string;
  installed: boolean;
}

export type ManagementTab = 'models' | 'agents' | 'skills' | 'general';

// --- Memory server types ---

export interface MemorySource {
  id: string;
  name: string;
  resource_type: string;
  path: string;
  enabled: boolean;
  include_patterns: string[];
  exclude_patterns: string[];
  latest_job?: MemoryIndexingJob | null;
  stats?: MemorySourceStats | null;
}

export interface MemorySourceStats {
  chunk_count: number;
  file_count: number;
  total_size_bytes: number;
}

export interface MemoryIndexingJob {
  id: string;
  source_id: string;
  source_name: string;
  source_type: string;
  status: string;
  started_at: string;
  finished_at?: string | null;
  files_indexed?: number | null;
  chunks_created?: number | null;
  total_files?: number | null;
  error?: string | null;
}

export interface MemorySearchResult {
  id: string;
  content: string;
  source_id: string;
  source_name: string;
  file_path: string;
  score: number;
  metadata?: Record<string, any>;
}

export interface MemoryServerStatus {
  status: string;
  message?: string | null;
  progress?: string | null;
}

// Plan mode types
export type PlanItemStatus = 'pending' | 'in_progress' | 'done' | 'skipped';
export type PlanStatus = 'planned' | 'approved' | 'executing' | 'completed';

export interface PlanItem {
  title: string;
  description?: string | null;
  status: PlanItemStatus;
}

export interface Plan {
  summary: string;
  items: PlanItem[];
  status: PlanStatus;
}
