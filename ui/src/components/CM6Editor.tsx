import React, { useMemo } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView } from '@codemirror/view';

export const CM6Editor: React.FC<{
  value: string;
  onChange: (value: string) => void;
  readOnly?: boolean;
}> = ({ value, onChange, readOnly = false }) => {
  const isDark = useMemo(() => {
    if (typeof window === 'undefined') return false;
    return document.documentElement.classList.contains('dark');
  }, []);

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      readOnly={readOnly}
      height="100%"
      theme={isDark ? oneDark : 'light'}
      extensions={[
        markdown({
          base: markdownLanguage,
          codeLanguages: languages,
        }),
        EditorView.lineWrapping,
      ]}
      basicSetup={{
        lineNumbers: true,
        foldGutter: true,
        highlightActiveLineGutter: true,
        highlightActiveLine: true,
        history: true,
      }}
    />
  );
};

