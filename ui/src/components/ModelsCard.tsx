import React from 'react';

import { cn } from '../lib/cn';
import type { ChatMessage, ModelInfo, OllamaPsResponse } from '../types';

export const ModelsCard: React.FC<{
  models: ModelInfo[];
  ollamaStatus: OllamaPsResponse | null;
  chatMessages: ChatMessage[];
  tokensPerSec?: number;
}> = ({ models, ollamaStatus, chatMessages, tokensPerSec }) => {
  const tps = Number.isFinite(Number(tokensPerSec)) ? Number(tokensPerSec) : 0;
  const tpsText = tps > 0 ? tps.toFixed(1) : '0.0';
  return (
      <div className="flex-1 p-4 overflow-y-auto text-xs space-y-3">
        {models.map((m) => {
          const isActive = ollamaStatus?.models.some((om) => om.name.includes(m.model) || m.model.includes(om.name));
          const activeInfo = ollamaStatus?.models.find((om) => om.name.includes(m.model) || m.model.includes(om.name));

          return (
            <div key={m.id} className="bg-slate-50 dark:bg-black/20 p-3 rounded-xl border border-slate-200 dark:border-white/5 space-y-2">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <div className={cn('w-1.5 h-1.5 rounded-full', isActive ? 'bg-green-500 animate-pulse' : 'bg-slate-500')} />
                  <span className="font-mono font-bold">{m.model}</span>
                </div>
                <span className="text-[9px] text-slate-500 uppercase tracking-wide">{m.provider}</span>
              </div>

              {activeInfo && (
                <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-[9px] text-slate-400 font-mono">
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span className="text-slate-500 dark:text-slate-400">PARAMS:</span>
                    <span className="text-slate-700 dark:text-slate-100 font-semibold">{activeInfo.details.parameter_size}</span>
                  </div>
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span className="text-slate-500 dark:text-slate-400">VRAM:</span>
                    <span className="text-slate-700 dark:text-slate-100 font-semibold">{(activeInfo.size_vram / 1024 / 1024 / 1024).toFixed(1)}GB</span>
                  </div>
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span className="text-slate-500 dark:text-slate-400">QUANT:</span>
                    <span className="text-slate-700 dark:text-slate-100 font-semibold">{activeInfo.details.quantization_level}</span>
                  </div>
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span className="text-slate-500 dark:text-slate-400">CONTEXT:</span>
                    <span className="text-slate-700 dark:text-slate-100 font-semibold">
                      {chatMessages.reduce((acc, msg) => acc + msg.text.length, 0).toLocaleString()}
                    </span>
                  </div>
                  <div className="flex justify-between border-b border-white/5 pb-0.5">
                    <span className="text-slate-500 dark:text-slate-400">TOK/S:</span>
                    <span className={cn(
                      'font-semibold',
                      isActive && tps > 0
                        ? 'text-emerald-600 dark:text-emerald-300'
                        : 'text-slate-700 dark:text-slate-100'
                    )}>
                      {isActive ? tpsText : '-'}
                    </span>
                  </div>
                </div>
              )}

              {!activeInfo && <div className="text-[9px] text-slate-600 italic">Model is currently idle</div>}
            </div>
          );
        })}
      </div>
  );
};
