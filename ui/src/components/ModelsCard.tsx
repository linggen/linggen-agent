import React from 'react';
import { Sparkles } from 'lucide-react';
import { cn } from '../lib/cn';
import type { ChatMessage, ModelInfo, OllamaPsResponse } from '../types';

export const ModelsCard: React.FC<{
  models: ModelInfo[];
  ollamaStatus: OllamaPsResponse | null;
  chatMessages: ChatMessage[];
}> = ({ models, ollamaStatus, chatMessages }) => {
  return (
    <section className="bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm flex flex-col overflow-hidden">
      <div className="px-4 py-2 border-b border-slate-200 dark:border-white/5 bg-slate-50 dark:bg-white/[0.02] flex items-center justify-between">
        <h3 className="text-[10px] font-bold uppercase tracking-widest text-purple-500 flex items-center gap-2">
          <Sparkles size={12} /> Models Status
        </h3>
        <span className="text-[10px] text-slate-400">Inference</span>
      </div>
      <div className="flex-1 p-4 overflow-y-auto text-xs space-y-3">
        {models.map((m) => {
          const isActive = ollamaStatus?.models.some((om) => om.name.includes(m.model) || m.model.includes(om.name));
          const activeInfo = ollamaStatus?.models.find((om) => om.name.includes(m.model) || m.model.includes(om.name));

          return (
            <div key={m.id} className="bg-slate-50 dark:bg-black/20 p-3 rounded-lg border border-slate-200 dark:border-white/5 space-y-2">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <div className={cn('w-1.5 h-1.5 rounded-full', isActive ? 'bg-green-500 animate-pulse' : 'bg-slate-500')} />
                  <span className="font-mono font-bold">{m.model}</span>
                </div>
                <span className="text-[8px] text-slate-500 uppercase">{m.provider}</span>
              </div>

              {activeInfo && (
                <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[9px] text-slate-400 font-mono">
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span>PARAMS:</span>
                    <span className="text-slate-200">{activeInfo.details.parameter_size}</span>
                  </div>
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span>VRAM:</span>
                    <span className="text-slate-200">{(activeInfo.size_vram / 1024 / 1024 / 1024).toFixed(1)}GB</span>
                  </div>
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span>QUANT:</span>
                    <span className="text-slate-200">{activeInfo.details.quantization_level}</span>
                  </div>
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span>CONTEXT:</span>
                    <span className="text-slate-200">
                      {chatMessages.reduce((acc, msg) => acc + msg.text.length, 0).toLocaleString()}
                    </span>
                  </div>
                </div>
              )}

              {!activeInfo && <div className="text-[9px] text-slate-600 italic">Model is currently idle</div>}
            </div>
          );
        })}
      </div>
    </section>
  );
};
