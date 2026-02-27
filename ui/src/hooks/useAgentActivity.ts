/**
 * Agent activity state: status badges, subagent tracking, token rate, run timing.
 */
import { useState, useEffect, useRef, useCallback } from 'react';
import type { ChatMessage } from '../types';
import { TOKEN_RATE_WINDOW_MS, TOKEN_RATE_IDLE_RESET_MS } from '../lib/messageUtils';

export type AgentStatusValue = 'idle' | 'model_loading' | 'thinking' | 'calling_tool' | 'working';

export function useAgentActivity(chatMessages: ChatMessage[]) {
  const [agentStatus, setAgentStatus] = useState<Record<string, AgentStatusValue>>({});
  const [agentStatusText, setAgentStatusText] = useState<Record<string, string>>({});
  const [agentContext, setAgentContext] = useState<Record<string, { tokens: number; messages: number; tokenLimit?: number }>>({});
  const [tokensPerSec, setTokensPerSec] = useState<number>(0);

  const isRunning = Object.values(agentStatus).some((s) => s !== 'idle');

  // Refs for tracking
  const runStartTsRef = useRef<Record<string, number>>({});
  const latestContextTokensRef = useRef<Record<string, number>>({});
  const subagentParentMapRef = useRef<Record<string, string>>({});
  const subagentStatsRef = useRef<Record<string, { toolCount: number; contextTokens: number }>>({});
  const tokenRateSamplesRef = useRef<Array<{ ts: number; tokens: number }>>([]);
  const lastTokenAtRef = useRef<number>(0);
  const lastAgentCharsRef = useRef<number>(0);
  const lastAgentCharsTsRef = useRef<number>(0);
  const hadGeneratingRef = useRef<boolean>(false);

  const recomputeTokenRate = useCallback((nowMs?: number) => {
    const now = nowMs ?? Date.now();
    const cutoff = now - TOKEN_RATE_WINDOW_MS;
    const pruned = tokenRateSamplesRef.current.filter((sample) => sample.ts >= cutoff);
    tokenRateSamplesRef.current = pruned;
    if (pruned.length === 0) {
      setTokensPerSec(0);
      return;
    }
    const totalTokens = pruned.reduce((sum, sample) => sum + sample.tokens, 0);
    const oldestTs = pruned[0]?.ts ?? now;
    const elapsedSec = Math.max((now - oldestTs) / 1000, 0.25);
    const rate = totalTokens / elapsedSec;
    setTokensPerSec(Number.isFinite(rate) ? rate : 0);
  }, []);

  // Estimate token rate from chat message text growth (fallback when no token SSE events)
  useEffect(() => {
    const now = Date.now();
    const agentChars = chatMessages.reduce((sum, msg) => {
      const from = msg.from || msg.role;
      if (from === 'user') return sum;
      return sum + String(msg.text || '').length;
    }, 0);
    const hasGeneratingAgent = chatMessages.some((msg) => {
      const from = msg.from || msg.role;
      return from !== 'user' && !!msg.isGenerating;
    });

    if (lastAgentCharsTsRef.current > 0) {
      const deltaChars = agentChars - lastAgentCharsRef.current;
      const elapsedMs = now - lastAgentCharsTsRef.current;
      const noRecentTokenEvents = now - lastTokenAtRef.current > 1_200;
      if (
        noRecentTokenEvents &&
        deltaChars > 0 &&
        elapsedMs > 0 &&
        (hasGeneratingAgent || hadGeneratingRef.current)
      ) {
        const tokens = Math.max(1, Math.floor((deltaChars + 3) / 4));
        tokenRateSamplesRef.current.push({ ts: now, tokens });
        lastTokenAtRef.current = now;
        recomputeTokenRate(now);
      }
    }

    lastAgentCharsRef.current = agentChars;
    lastAgentCharsTsRef.current = now;
    hadGeneratingRef.current = hasGeneratingAgent;
  }, [chatMessages, recomputeTokenRate]);

  // Periodic token rate decay
  useEffect(() => {
    const timer = window.setInterval(() => {
      const now = Date.now();
      if (lastTokenAtRef.current === 0 || now - lastTokenAtRef.current > TOKEN_RATE_IDLE_RESET_MS) {
        tokenRateSamplesRef.current = [];
        setTokensPerSec(0);
        return;
      }
      recomputeTokenRate(now);
    }, 500);
    return () => window.clearInterval(timer);
  }, [recomputeTokenRate]);

  const resetStatus = useCallback(() => {
    setAgentStatus({});
    setAgentStatusText({});
  }, []);

  return {
    agentStatus,
    setAgentStatus,
    agentStatusText,
    setAgentStatusText,
    agentContext,
    setAgentContext,
    tokensPerSec,
    isRunning,
    runStartTsRef,
    latestContextTokensRef,
    subagentParentMapRef,
    subagentStatsRef,
    resetStatus,
  };
}
