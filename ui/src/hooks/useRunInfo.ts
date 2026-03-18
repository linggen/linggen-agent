/**
 * Derived run information — sorted runs, run IDs by agent, run history.
 */
import { useMemo } from 'react';
import type { AgentRunInfo, AgentRunSummary } from '../types';
import { useAgentStore } from '../stores/agentStore';

export function useRunInfo() {
  const agentRuns = useAgentStore((s) => s.agentRuns);
  const agents = useAgentStore((s) => s.agents);

  const sortedAgentRuns = useMemo(() => {
    const statusScore = (status: string) => (status === 'running' ? 1 : 0);
    return [...agentRuns].sort(
      (a, b) => statusScore(b.status) - statusScore(a.status) || Number(b.started_at || 0) - Number(a.started_at || 0),
    );
  }, [agentRuns]);

  const mainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  const subagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  const runningMainRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  const runningSubagentRunIds = useMemo(() => {
    const out: Record<string, string> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id || run.status !== 'running') continue;
      if (!out[agentId]) out[agentId] = run.run_id;
    }
    return out;
  }, [sortedAgentRuns]);

  const mainRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);

  const subagentRunHistory = useMemo(() => {
    const out: Record<string, AgentRunInfo[]> = {};
    for (const run of sortedAgentRuns) {
      const agentId = run.agent_id.toLowerCase();
      if (!run.parent_run_id) continue;
      if (!out[agentId]) out[agentId] = [];
      out[agentId].push(run);
    }
    return out;
  }, [sortedAgentRuns]);

  const agentRunSummary = useMemo(() => {
    const out: Record<string, AgentRunSummary> = {};
    for (const agent of agents) {
      const agentId = agent.name.toLowerCase();
      const latest = mainRunHistory[agentId]?.[0];
      if (!latest) continue;
      const children = sortedAgentRuns.filter((run) => run.parent_run_id === latest.run_id);
      const timelineEvents = 1 + (latest.ended_at ? 1 : 0) + children.reduce((count, child) => count + 1 + (child.ended_at ? 1 : 0), 0);
      const lastEventAt = Math.max(
        Number(latest.ended_at || 0), Number(latest.started_at || 0),
        ...children.flatMap((child) => [Number(child.started_at || 0), Number(child.ended_at || 0)]),
      );
      out[agentId] = {
        run_id: latest.run_id,
        status: latest.status,
        started_at: latest.started_at,
        ended_at: latest.ended_at,
        child_count: children.length,
        timeline_events: timelineEvents,
        last_event_at: lastEventAt,
      };
    }
    return out;
  }, [agents, mainRunHistory, sortedAgentRuns]);

  return {
    sortedAgentRuns,
    mainRunIds,
    subagentRunIds,
    runningMainRunIds,
    runningSubagentRunIds,
    mainRunHistory,
    subagentRunHistory,
    agentRunSummary,
  };
}
