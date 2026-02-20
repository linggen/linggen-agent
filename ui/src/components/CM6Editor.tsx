import React, { useMemo } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView } from '@codemirror/view';
import {
  livePreviewPlugin,
  livePreviewTheme,
  livePreviewLightTheme,
} from './cm6-live-preview';

export const CM6Editor: React.FC<{
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
  livePreview?: boolean;
}> = ({ value, onChange, readOnly = false, livePreview = false }) => {
  const isDark = useMemo(() => {
    if (typeof window === 'undefined') return false;
    return document.documentElement.classList.contains('dark');
  }, []);

  const extensions = useMemo(() => {
    const exts = [
      markdown({
        base: markdownLanguage,
        codeLanguages: languages,
      }),
      EditorView.lineWrapping,
    ];
    if (livePreview) {
      exts.push(livePreviewPlugin);
      exts.push(livePreviewTheme);
      if (!isDark) {
        exts.push(livePreviewLightTheme);
      }
    }
    return exts;
  }, [livePreview, isDark]);

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      readOnly={readOnly}
      height="100%"
      theme={isDark ? oneDark : 'light'}
      extensions={extensions}
      basicSetup={{
        lineNumbers: !livePreview,
        foldGutter: true,
        highlightActiveLineGutter: !livePreview,
        highlightActiveLine: true,
        history: true,
      }}
    />
  );
};
