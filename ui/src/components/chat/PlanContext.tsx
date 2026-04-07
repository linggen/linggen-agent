/**
 * React context for plan-related props.
 * Eliminates props drilling through ChatPanel → ChatMessageRow → AgentMessage → PlanBlock.
 */
import { createContext, useContext, useMemo } from 'react';
import type React from 'react';

export interface PlanContextValue {
  pendingPlanAgentId?: string | null;
  agentContext?: Record<string, { tokens: number; messages: number; tokenLimit?: number }>;
  onApprovePlan?: () => void;
  onRejectPlan?: () => void;
  onEditPlan?: (text: string) => void;
  inputRef: React.RefObject<HTMLTextAreaElement | null>;
}

const PlanCtx = createContext<PlanContextValue | null>(null);

export const PlanProvider: React.FC<PlanContextValue & { children: React.ReactNode }> = ({
  children,
  pendingPlanAgentId,
  agentContext,
  onApprovePlan,
  onRejectPlan,
  onEditPlan,
  inputRef,
}) => {
  const value = useMemo(
    () => ({ pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef }),
    [pendingPlanAgentId, agentContext, onApprovePlan, onRejectPlan, onEditPlan, inputRef]
  );
  return <PlanCtx.Provider value={value}>{children}</PlanCtx.Provider>;
};

export const usePlanContext = (): PlanContextValue => {
  const ctx = useContext(PlanCtx);
  if (!ctx) throw new Error('usePlanContext must be used within PlanProvider');
  return ctx;
};
