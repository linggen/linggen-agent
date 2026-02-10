export interface ChatMessage {
  role: 'user' | 'agent' | 'lead' | 'coder';
  from?: string;
  to?: string;
  text: string;
  timestamp: string;
  timestampMs?: number;
  isGenerating?: boolean;
  activitySummary?: string;
  activityEntries?: string[];
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

export interface LeadState {
  active_lead_task: [any, string] | null;
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

export interface SkillInfo {
  name: string;
  description: string;
  source: { type: string };
}

export interface AgentInfo {
  name: string;
  description: string;
  kind?: 'main' | 'subagent';
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
  agent_kind: 'main' | 'subagent' | string;
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
