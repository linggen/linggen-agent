export type TimelineEvent = {
  ts: number;
  label: string;
  detail?: string;
  kind: 'run' | 'subagent' | 'tool' | 'task';
};

export type ToolIntent = {
  name: string;
  detail?: string;
};

export type MessagePhase = 'thinking' | 'working' | 'streaming' | 'done';

export interface SpecialBlockProps {
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: (clearContext: boolean) => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  inputRef?: React.RefObject<HTMLTextAreaElement | null>;
}
