/** API client for Linggen server */

export async function createSession(baseUrl: string, title: string, skill?: string) {
  const res = await fetch(`${baseUrl}/api/sessions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ title, skill }),
  });
  if (!res.ok) throw new Error(`Failed to create session: ${res.statusText}`);
  return res.json() as Promise<{ id: string }>;
}

export async function sendChat(
  baseUrl: string,
  opts: {
    skillName?: string;
    agentId: string;
    sessionId: string;
    modelId: string;
    message: string;
  }
) {
  const res = await fetch(`${baseUrl}/api/chat`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      project_root: '',
      agent_id: opts.agentId,
      session_id: opts.sessionId,
      skill_name: opts.skillName || null,
      model_id: opts.modelId,
      message: opts.message,
    }),
  });
  if (!res.ok) throw new Error(`Chat failed: ${res.statusText}`);
  return res.json();
}

export async function fetchModels(baseUrl: string) {
  const res = await fetch(`${baseUrl}/api/models`);
  if (!res.ok) throw new Error(`Failed to fetch models`);
  return res.json() as Promise<{ id: string; provider: string }[]>;
}

export async function fetchDefaultModel(baseUrl: string) {
  const res = await fetch(`${baseUrl}/api/config`);
  if (!res.ok) return null;
  const config = await res.json();
  return config?.routing?.default_models?.[0] ?? null;
}
