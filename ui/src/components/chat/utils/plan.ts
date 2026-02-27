export const PLAN_STATUS_COLOR: Record<string, string> = {
  planned: 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400',
  approved: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400',
  executing: 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
  completed: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400',
};

export const PLAN_ITEM_ICON: Record<string, string> = {
  pending: '\u25CB',
  in_progress: '\u25D1',
  done: '\u25CF',
  skipped: '\u2298',
};

export const PLAN_ITEM_COLOR: Record<string, string> = {
  pending: 'text-slate-400',
  in_progress: 'text-blue-500',
  done: 'text-emerald-500',
  skipped: 'text-slate-300 dark:text-slate-600',
};
