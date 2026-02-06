export interface ChatMessage {
  role: 'user' | 'agent' | 'lead' | 'coder';
  from?: string;
  to?: string;
  text: string;
  timestamp: string;
  isGenerating?: boolean;
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
  children?: Record<string, AgentTreeItem>;
}

export interface SkillInfo {
  name: string;
  description: string;
  source: { type: string };
}

export interface AgentInfo {
  name: string;
  description: string;
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
