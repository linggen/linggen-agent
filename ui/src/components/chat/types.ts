export type MessagePhase = 'thinking' | 'working' | 'streaming' | 'done';

export interface SpecialBlockProps {
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: (clearContext: boolean) => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  inputRef?: React.RefObject<HTMLTextAreaElement | null>;
}
