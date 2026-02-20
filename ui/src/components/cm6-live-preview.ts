/**
 * Live Preview Plugin for CodeMirror 6
 *
 * Hides markdown syntax and shows rendered content inline,
 * similar to Obsidian's Live Preview mode.
 *
 * When cursor is on a line, syntax is shown; when cursor moves away,
 * the markdown is rendered.
 *
 * Ported from linggen-memory's cm6-live-preview.ts with theme
 * adjustments for linggen-agent's Tailwind v4 UI.
 */

import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from '@codemirror/view';
import { syntaxTree } from '@codemirror/language';
import { RangeSetBuilder } from '@codemirror/state';

// Lazy mermaid import to avoid blocking
let mermaidInstance: typeof import('mermaid').default | null = null;
let mermaidInitialized = false;

async function getMermaid() {
  if (!mermaidInstance) {
    try {
      const mermaidModule = await import('mermaid');
      mermaidInstance = mermaidModule.default;
      if (!mermaidInitialized) {
        const isDark =
          document.documentElement.classList.contains('dark') ||
          window.matchMedia?.('(prefers-color-scheme: dark)').matches;
        mermaidInstance.initialize({
          startOnLoad: false,
          theme: isDark ? 'dark' : 'default',
          securityLevel: 'loose',
        });
        mermaidInitialized = true;
      }
    } catch (err) {
      console.error('Failed to load mermaid:', err);
      return null;
    }
  }
  return mermaidInstance;
}

// === Widget Classes for replaced content ===

// Track which mermaid blocks are in "edit raw text" mode.
const mermaidEditBlocks = new Set<number>();

class MermaidWidget extends WidgetType {
  private code: string;
  private id: string;
  private blockPos: number;

  constructor(code: string, blockPos: number) {
    super();
    this.code = code;
    this.blockPos = blockPos;
    this.id = `mermaid-${Math.random().toString(36).substr(2, 9)}`;
  }

  toDOM(view: EditorView) {
    const container = document.createElement('div');
    container.className = 'cm-mermaid-container';
    container.style.position = 'relative';
    container.style.margin = '8px 0';

    const editBtn = document.createElement('button');
    editBtn.style.cssText =
      'position:absolute;top:8px;right:8px;background:rgba(59,130,246,0.8);border:none;color:white;padding:2px 8px;border-radius:4px;cursor:pointer;font-size:12px;z-index:10;opacity:0;transition:opacity 0.2s';
    editBtn.innerHTML = '&lt;/&gt;';
    editBtn.title = 'Edit this block';

    container.addEventListener('mouseenter', () => {
      editBtn.style.opacity = '1';
    });
    container.addEventListener('mouseleave', () => {
      editBtn.style.opacity = '0';
    });

    editBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      mermaidEditBlocks.add(this.blockPos);
      view.dispatch({
        selection: { anchor: this.blockPos },
        scrollIntoView: true,
      });
      view.focus();
    });

    const diagramContainer = document.createElement('div');
    diagramContainer.style.cssText =
      'display:flex;justify-content:center;padding:16px;background:rgba(0,0,0,0.08);border-radius:8px;min-height:80px';
    diagramContainer.innerHTML =
      '<div style="color:#94a3b8;padding:16px">Loading diagram...</div>';

    container.appendChild(editBtn);
    container.appendChild(diagramContainer);

    this.renderMermaid(diagramContainer);

    return container;
  }

  private async renderMermaid(container: HTMLElement) {
    try {
      const mermaid = await getMermaid();
      if (!mermaid) {
        container.innerHTML =
          '<div style="color:#ef4444;padding:8px">Mermaid not available</div>';
        return;
      }
      const cleanCode = this.code.trim();
      const { svg } = await mermaid.render(this.id, cleanCode);
      container.innerHTML = svg;
    } catch (err) {
      container.innerHTML = `<div style="color:#ef4444;padding:8px">Mermaid Error: ${err instanceof Error ? err.message : String(err)}</div>`;
    }
  }

  eq(other: MermaidWidget) {
    return this.code === other.code && this.blockPos === other.blockPos;
  }

  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  ignoreEvent(_event: Event) {
    return false;
  }
}

class HorizontalRuleWidget extends WidgetType {
  toDOM() {
    const hr = document.createElement('hr');
    hr.className = 'cm-hr-widget';
    return hr;
  }
}

class CheckboxWidget extends WidgetType {
  private checked: boolean;

  constructor(checked: boolean) {
    super();
    this.checked = checked;
  }

  toDOM() {
    const span = document.createElement('span');
    span.className = `cm-checkbox-widget ${this.checked ? 'checked' : ''}`;
    span.textContent = this.checked ? '\u2611' : '\u2610';
    return span;
  }
}

class BulletWidget extends WidgetType {
  toDOM() {
    const span = document.createElement('span');
    span.className = 'cm-list-bullet';
    span.textContent = '\u2022 ';
    return span;
  }
}

// === Decoration Classes ===

const hiddenMarkDecoration = Decoration.mark({ class: 'cm-hidden-syntax' });
const boldDecoration = Decoration.mark({ class: 'cm-rendered-strong' });
const italicDecoration = Decoration.mark({ class: 'cm-rendered-emphasis' });
const strikeDecoration = Decoration.mark({ class: 'cm-rendered-strike' });
const linkDecoration = Decoration.mark({ class: 'cm-rendered-link' });
const codeDecoration = Decoration.mark({ class: 'cm-rendered-code' });
const blockquoteDecoration = Decoration.line({ class: 'cm-blockquote-line' });

// === Helper functions ===

function getActiveLines(view: EditorView): Set<number> {
  const activeLines = new Set<number>();
  for (const range of view.state.selection.ranges) {
    const startLine = view.state.doc.lineAt(range.from).number;
    const endLine = view.state.doc.lineAt(range.to).number;
    for (let i = startLine; i <= endLine; i++) {
      activeLines.add(i);
    }
  }
  return activeLines;
}

// === The main ViewPlugin ===

export const livePreviewPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = this.buildDecorations(view);
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet || update.viewportChanged) {
        this.decorations = this.buildDecorations(update.view);
      }
    }

    buildDecorations(view: EditorView): DecorationSet {
      const activeLines = getActiveLines(view);
      const doc = view.state.doc;

      const decorations: {
        from: number;
        to: number;
        decoration: Decoration;
      }[] = [];

      // === Standard markdown live preview decorations ===
      for (const { from, to } of view.visibleRanges) {
        syntaxTree(view.state).iterate({
          from,
          to,
          enter: (node) => {
            const line = doc.lineAt(node.from);
            const isActiveLine = activeLines.has(line.number);

            // Show raw markdown on active lines
            if (isActiveLine) return;

            const nodeType = node.name;

            // Headers — hide # marks
            if (
              nodeType.startsWith('ATXHeading') ||
              nodeType === 'HeaderMark'
            ) {
              if (nodeType === 'HeaderMark') {
                decorations.push({
                  from: node.from,
                  to: node.to + 1,
                  decoration: hiddenMarkDecoration,
                });
              }
            }

            // Bold **text** or __text__
            if (nodeType === 'StrongEmphasis') {
              const text = doc.sliceString(node.from, node.to);
              const marker = text.startsWith('**') ? '**' : '__';
              decorations.push({
                from: node.from,
                to: node.from + marker.length,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.to - marker.length,
                to: node.to,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.from + marker.length,
                to: node.to - marker.length,
                decoration: boldDecoration,
              });
            }

            // Italic *text* or _text_
            if (nodeType === 'Emphasis') {
              const text = doc.sliceString(node.from, node.to);
              const marker = text.startsWith('*') ? '*' : '_';
              decorations.push({
                from: node.from,
                to: node.from + marker.length,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.to - marker.length,
                to: node.to,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.from + marker.length,
                to: node.to - marker.length,
                decoration: italicDecoration,
              });
            }

            // Strikethrough ~~text~~
            if (nodeType === 'Strikethrough') {
              decorations.push({
                from: node.from,
                to: node.from + 2,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.to - 2,
                to: node.to,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.from + 2,
                to: node.to - 2,
                decoration: strikeDecoration,
              });
            }

            // Inline code `code`
            if (nodeType === 'InlineCode') {
              decorations.push({
                from: node.from,
                to: node.from + 1,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.to - 1,
                to: node.to,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.from + 1,
                to: node.to - 1,
                decoration: codeDecoration,
              });
            }

            // Links [text](url)
            if (nodeType === 'Link') {
              const text = doc.sliceString(node.from, node.to);
              const linkMatch = text.match(/^\[([^\]]*)\]\(([^)]*)\)$/);
              if (linkMatch) {
                const textStart = node.from + 1;
                const textEnd = node.from + 1 + linkMatch[1].length;
                decorations.push({
                  from: node.from,
                  to: node.from + 1,
                  decoration: hiddenMarkDecoration,
                });
                decorations.push({
                  from: textEnd,
                  to: node.to,
                  decoration: hiddenMarkDecoration,
                });
                decorations.push({
                  from: textStart,
                  to: textEnd,
                  decoration: linkDecoration,
                });
              }
            }

            // Blockquotes > — hide QuoteMark
            if (nodeType === 'QuoteMark') {
              decorations.push({
                from: node.from,
                to: node.to + 1,
                decoration: hiddenMarkDecoration,
              });
            }

            if (nodeType === 'Blockquote') {
              decorations.push({
                from: line.from,
                to: line.from,
                decoration: blockquoteDecoration,
              });
            }

            // Horizontal rule ---
            if (nodeType === 'HorizontalRule') {
              decorations.push({
                from: node.from,
                to: node.to,
                decoration: Decoration.replace({
                  widget: new HorizontalRuleWidget(),
                }),
              });
            }

            // List markers
            if (nodeType === 'ListMark') {
              decorations.push({
                from: node.from,
                to: node.to + 1,
                decoration: Decoration.replace({
                  widget: new BulletWidget(),
                }),
              });
            }

            // Task list checkboxes
            if (nodeType === 'TaskMarker') {
              const text = doc.sliceString(node.from, node.to);
              const isChecked = text.includes('x') || text.includes('X');
              decorations.push({
                from: node.from,
                to: node.to,
                decoration: Decoration.replace({
                  widget: new CheckboxWidget(isChecked),
                }),
              });
            }
          },
        });
      }

      // === Mermaid fenced code blocks ===
      const fullText = doc.toString();
      const mermaidRegex = /```mermaid\s*\n([\s\S]*?)```/g;
      let match: RegExpExecArray | null;

      while ((match = mermaidRegex.exec(fullText)) !== null) {
        const blockCode = match[1];
        const blockStart = match.index;
        const blockEnd = match.index + match[0].length;

        if (mermaidEditBlocks.has(blockStart)) {
          let stillEditing = false;
          for (const range of view.state.selection.ranges) {
            if (range.from <= blockEnd && range.to >= blockStart) {
              stillEditing = true;
              break;
            }
          }
          if (!stillEditing) {
            mermaidEditBlocks.delete(blockStart);
          } else {
            continue;
          }
        }

        decorations.push({
          from: blockStart,
          to: blockStart,
          decoration: Decoration.widget({
            widget: new MermaidWidget(blockCode, blockStart),
          }),
        });

        const startLine = doc.lineAt(blockStart).number;
        const endLine = doc.lineAt(blockEnd).number;
        for (let lineNo = startLine; lineNo <= endLine; lineNo++) {
          const line = doc.line(lineNo);
          decorations.push({
            from: line.from,
            to: line.from,
            decoration: Decoration.line({
              class: 'cm-mermaid-hidden-line',
            }),
          });
        }
      }

      // Sort decorations by position
      decorations.sort((a, b) => a.from - b.from || a.to - b.to);

      const builder = new RangeSetBuilder<Decoration>();
      for (const { from, to, decoration } of decorations) {
        try {
          builder.add(from, to, decoration);
        } catch {
          // Skip invalid ranges
        }
      }

      return builder.finish();
    }
  },
  {
    decorations: (v) => v.decorations,
  }
);

// === Theme for live preview elements ===

export const livePreviewTheme = EditorView.theme({
  '.cm-hidden-syntax': {
    fontSize: '0',
    width: '0',
    display: 'none',
  },

  // Headers — rendered via CM6 markdown language support
  '.cm-header-line.cm-header-1': {
    fontSize: '1.8em',
    fontWeight: 'bold',
    lineHeight: '1.3',
    color: '#e2e8f0',
  },
  '.cm-header-line.cm-header-2': {
    fontSize: '1.5em',
    fontWeight: 'bold',
    lineHeight: '1.3',
    color: '#e2e8f0',
  },
  '.cm-header-line.cm-header-3': {
    fontSize: '1.3em',
    fontWeight: 'bold',
    lineHeight: '1.3',
    color: '#e2e8f0',
  },

  '.cm-rendered-strong': {
    fontWeight: 'bold',
    color: '#e2e8f0',
  },
  '.cm-rendered-emphasis': {
    fontStyle: 'italic',
    color: '#94a3b8',
  },
  '.cm-rendered-strike': {
    textDecoration: 'line-through',
    color: '#64748b',
  },
  '.cm-rendered-link': {
    color: '#60a5fa',
    textDecoration: 'underline',
    cursor: 'pointer',
  },
  '.cm-rendered-code': {
    backgroundColor: 'rgba(59,130,246,0.15)',
    color: '#60a5fa',
    padding: '2px 6px',
    borderRadius: '4px',
    fontFamily:
      'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    fontSize: '0.9em',
  },
  '.cm-blockquote-line': {
    borderLeft: '3px solid #3b82f6',
    paddingLeft: '12px',
    color: '#64748b',
    fontStyle: 'italic',
  },
  '.cm-list-bullet': {
    color: '#3b82f6',
    fontWeight: 'bold',
    marginRight: '8px',
  },
  '.cm-checkbox-widget': {
    display: 'inline-block',
    width: '18px',
    height: '18px',
    marginRight: '8px',
    fontSize: '16px',
    color: '#64748b',
    cursor: 'pointer',
  },
  '.cm-checkbox-widget.checked': {
    color: '#22c55e',
  },
  '.cm-hr-widget': {
    display: 'block',
    border: 'none',
    borderTop: '1px solid rgba(148,163,184,0.2)',
    margin: '16px 0',
  },
  '.cm-mermaid-hidden-line': {
    height: '0 !important',
    padding: '0 !important',
    margin: '0 !important',
    overflow: 'hidden !important',
    opacity: '0',
    fontSize: '0',
    lineHeight: '0',
  },
});

// Light-mode overrides
export const livePreviewLightTheme = EditorView.theme({
  '.cm-header-line.cm-header-1': { color: '#1e293b' },
  '.cm-header-line.cm-header-2': { color: '#1e293b' },
  '.cm-header-line.cm-header-3': { color: '#1e293b' },
  '.cm-rendered-strong': { color: '#1e293b' },
  '.cm-rendered-emphasis': { color: '#475569' },
  '.cm-rendered-strike': { color: '#94a3b8' },
  '.cm-rendered-link': { color: '#2563eb' },
  '.cm-rendered-code': {
    backgroundColor: 'rgba(37,99,235,0.1)',
    color: '#2563eb',
  },
  '.cm-blockquote-line': { borderLeft: '3px solid #2563eb', color: '#94a3b8' },
  '.cm-list-bullet': { color: '#2563eb' },
});
