/** SSE connection manager */

export type SSECallback = (data: Record<string, unknown>) => void;

export function connectSSE(
  baseUrl: string,
  sessionId: string,
  onEvent: SSECallback
): EventSource {
  const url = `${baseUrl}/api/events?session_id=${encodeURIComponent(sessionId)}`;
  const es = new EventSource(url);

  es.onmessage = (e) => {
    try {
      const data = JSON.parse(e.data);
      onEvent(data);
    } catch {
      // ignore parse errors
    }
  };

  es.onerror = () => {
    // EventSource auto-reconnects
  };

  return es;
}
