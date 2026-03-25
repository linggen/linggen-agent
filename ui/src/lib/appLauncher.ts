/**
 * Open a skill app in a new browser tab.
 *
 * Local mode: opens the URL directly.
 * Remote mode: opens via ConnectPage with ?app= parameter (handled by handleClickSkill/handleAppLaunched).
 *
 * This module is kept for backward compatibility but the main logic
 * is now in App.tsx (handleClickSkill) and eventDispatcher.ts (handleAppLaunched).
 */

export async function openAppInNewTab(url: string): Promise<void> {
  window.open(url, '_blank');
}
