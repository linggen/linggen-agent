/** Lightweight markdown renderer — no external deps, handles common patterns */

export function MarkdownContent({ text }: { text: string }) {
  const html = renderMarkdown(text);
  return <div className="lc-md" dangerouslySetInnerHTML={{ __html: html }} />;
}

function renderMarkdown(text: string): string {
  // Escape HTML
  let html = text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');

  // Code blocks (```...```)
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, (_m, lang, code) => {
    return `<pre class="lc-codeblock"><code class="${lang}">${code.trim()}</code></pre>`;
  });

  // Process lines for block elements
  const lines = html.split('\n');
  const out: string[] = [];
  let inList = false;
  let listTag = 'ul';

  for (const line of lines) {
    // Headings
    const hMatch = line.match(/^(#{1,4})\s+(.+)$/);
    if (hMatch) {
      if (inList) { out.push(`</${listTag}>`); inList = false; }
      const lv = hMatch[1].length;
      out.push(`<h${lv} class="lc-h">${inlineFmt(hMatch[2])}</h${lv}>`);
      continue;
    }

    // Unordered list
    const ulMatch = line.match(/^\s*[-*]\s+(.+)$/);
    if (ulMatch) {
      if (!inList || listTag !== 'ul') {
        if (inList) out.push(`</${listTag}>`);
        out.push('<ul class="lc-list">');
        inList = true; listTag = 'ul';
      }
      out.push(`<li>${inlineFmt(ulMatch[1])}</li>`);
      continue;
    }

    // Ordered list
    const olMatch = line.match(/^\s*\d+\.\s+(.+)$/);
    if (olMatch) {
      if (!inList || listTag !== 'ol') {
        if (inList) out.push(`</${listTag}>`);
        out.push('<ol class="lc-list">');
        inList = true; listTag = 'ol';
      }
      out.push(`<li>${inlineFmt(olMatch[1])}</li>`);
      continue;
    }

    if (inList) { out.push(`</${listTag}>`); inList = false; }

    if (line.trim() === '') {
      out.push('<br>');
    } else {
      out.push(`<p class="lc-p">${inlineFmt(line)}</p>`);
    }
  }
  if (inList) out.push(`</${listTag}>`);

  return out.join('');
}

function inlineFmt(text: string): string {
  return text
    .replace(/`([^`]+)`/g, '<code class="lc-code">$1</code>')
    .replace(/\*\*\*(.+?)\*\*\*/g, '<strong><em>$1</em></strong>')
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/\*(.+?)\*/g, '<em>$1</em>')
    .replace(/~~(.+?)~~/g, '<del>$1</del>');
}
