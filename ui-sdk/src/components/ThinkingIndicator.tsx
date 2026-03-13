import { useEffect, useRef, useState } from 'react';

const VERBS = ['Thinking', 'Pondering', 'Analyzing', 'Considering', 'Brewing'];

export function ThinkingIndicator() {
  const [elapsed, setElapsed] = useState(0);
  const verbRef = useRef(VERBS[Math.floor(Math.random() * VERBS.length)]);
  const startRef = useRef(Date.now());

  useEffect(() => {
    const timer = setInterval(() => {
      setElapsed(Math.floor((Date.now() - startRef.current) / 1000));
    }, 500);
    return () => clearInterval(timer);
  }, []);

  const timeStr = elapsed < 60
    ? `${elapsed}s`
    : `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, '0')}`;

  return (
    <div className="lc-thinking">
      <span className="lc-thinking-label">{verbRef.current}</span>
      <span className="lc-thinking-dots">
        <span className="lc-dot" />
        <span className="lc-dot" />
        <span className="lc-dot" />
      </span>
      <span className="lc-thinking-time">{timeStr}</span>
    </div>
  );
}
