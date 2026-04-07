/**
 * Non-reactive agent activity tracker.
 *
 * Tracks ephemeral, high-frequency data that does NOT need to trigger React
 * re-renders: run start timestamps, context token snapshots, subagent
 * parent/child mappings, tool counts, and token-rate samples.
 *
 * Previously these lived as `_`-prefixed fields on the Zustand agentStore
 * and were mutated directly by eventDispatcher — bypassing Zustand's
 * reactivity.  Moving them here makes the intent explicit: this is plain
 * mutable state, not React state.
 */

export interface SubagentStats {
  toolCount: number;
  contextTokens: number;
}

class AgentTracker {
  /** When each session's current run started (epoch ms). */
  runStartTs: Record<string, number> = {};

  /** Latest estimated context-window tokens per session. */
  latestContextTokens: Record<string, number> = {};

  /** Maps lowercase subagent ID → lowercase parent agent ID. */
  subagentParentMap: Record<string, string> = {};

  /** Per-subagent tool count & context tokens. */
  subagentStats: Record<string, SubagentStats> = {};

  /** Sliding-window samples for tokens/sec calculation. */
  tokenRateSamples: Array<{ ts: number; tokens: number }> = [];

  /** Timestamp of the last token event (epoch ms). */
  lastTokenAt = 0;

  // -- Session lifecycle ----------------------------------------------------

  /** Reset all tracking state. Call on session switch or chat clear. */
  reset(): void {
    this.runStartTs = {};
    this.latestContextTokens = {};
    this.subagentParentMap = {};
    this.subagentStats = {};
    this.tokenRateSamples = [];
    this.lastTokenAt = 0;
  }

  // -- Run tracking --------------------------------------------------------

  ensureRunStarted(sid: string): void {
    if (!this.runStartTs[sid]) {
      this.runStartTs[sid] = Date.now();
    }
  }

  clearRun(sid: string): { elapsed?: number; contextTokens?: number } {
    const startTs = this.runStartTs[sid];
    const elapsed = startTs ? Date.now() - startTs : undefined;
    const ctxTokens = this.latestContextTokens[sid] || undefined;
    delete this.runStartTs[sid];
    delete this.latestContextTokens[sid];
    return { elapsed, contextTokens: ctxTokens };
  }

  // -- Subagent tracking ---------------------------------------------------

  registerSubagent(subagentId: string, parentId: string): void {
    this.subagentParentMap[subagentId.toLowerCase()] = parentId;
    this.subagentStats[subagentId.toLowerCase()] = { toolCount: 0, contextTokens: 0 };
  }

  unregisterSubagent(subagentId: string): void {
    delete this.subagentParentMap[subagentId.toLowerCase()];
    delete this.subagentStats[subagentId.toLowerCase()];
  }

  getParent(agentId: string): string | undefined {
    return this.subagentParentMap[agentId.toLowerCase()];
  }

  getStats(agentId: string): SubagentStats | undefined {
    return this.subagentStats[agentId.toLowerCase()];
  }

  incrementToolCount(agentId: string): number {
    const stats = this.subagentStats[agentId.toLowerCase()];
    if (stats) {
      stats.toolCount += 1;
      return stats.toolCount;
    }
    return 0;
  }

  setSubagentContextTokens(agentId: string, tokens: number): void {
    const stats = this.subagentStats[agentId.toLowerCase()];
    if (stats) stats.contextTokens = tokens;
  }

  // -- Token rate ----------------------------------------------------------

  recordTokenSample(tokens: number): void {
    this.lastTokenAt = Date.now();
    this.tokenRateSamples.push({ ts: this.lastTokenAt, tokens });
  }

  pruneAndComputeRate(windowMs: number, nowMs?: number): number {
    const now = nowMs ?? Date.now();
    const cutoff = now - windowMs;
    this.tokenRateSamples = this.tokenRateSamples.filter((s) => s.ts >= cutoff);
    if (this.tokenRateSamples.length === 0) return 0;
    const totalTokens = this.tokenRateSamples.reduce((sum, s) => sum + s.tokens, 0);
    const oldestTs = this.tokenRateSamples[0]?.ts ?? now;
    const elapsedSec = Math.max((now - oldestTs) / 1000, 0.25);
    const rate = totalTokens / elapsedSec;
    return Number.isFinite(rate) ? rate : 0;
  }
}

/** Singleton instance shared by eventDispatcher and agentStore. */
export const agentTracker = new AgentTracker();
