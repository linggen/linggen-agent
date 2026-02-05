import React, { useEffect, useRef } from 'react';
import { FileText, X } from 'lucide-react';

export const FilePreview: React.FC<{
  selectedFilePath: string | null;
  selectedFileContent: string | null;
  onClose: () => void;
}> = ({ selectedFilePath, selectedFileContent, onClose }) => {
  const panelRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  if (!selectedFilePath) return null;

  return (
    <div className="fixed inset-0 z-[80] flex items-center justify-center">
      <button
        className="absolute inset-0 bg-black/40 backdrop-blur-[2px]"
        onClick={onClose}
        aria-label="Close file preview"
      />
      <div
        ref={panelRef}
        className="relative w-[min(980px,92vw)] h-[min(70vh,720px)] bg-white dark:bg-[#141414] rounded-2xl border border-slate-200 dark:border-white/10 shadow-2xl overflow-hidden flex flex-col"
        role="dialog"
        aria-modal="true"
      >
        <div className="px-4 py-2 border-b border-slate-200 dark:border-white/5 bg-slate-50 dark:bg-white/[0.02] flex items-center justify-between">
          <div className="flex items-center gap-2">
            <FileText size={14} className="text-slate-400" />
            <span className="text-xs font-mono text-slate-600">{selectedFilePath}</span>
          </div>
          <button
            onClick={onClose}
            className="p-1.5 rounded-lg text-slate-500 hover:bg-slate-100 dark:hover:bg-white/5"
            title="Close"
          >
            <X size={14} />
          </button>
        </div>
        <div className="flex-1 p-4 font-mono text-xs overflow-auto bg-slate-900 text-slate-300 whitespace-pre">
          {selectedFileContent || '// No file selected'}
        </div>
      </div>
    </div>
  );
};
