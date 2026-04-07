/**
 * Client-side image cache for chat messages.
 *
 * Images are ephemeral (never persisted to disk), so we cache them here
 * keyed by "from::normalizedText" to survive merges and state reloads.
 */
import type { ChatMessage } from '../types';
import { normalizeMessageTextForDedup } from './messageUtils';

const _cache = new Map<string, string[]>();

function cacheKey(from: string, text: string): string {
  return `${from}::${normalizeMessageTextForDedup(text)}`;
}

/** Store images from a message into the cache. */
export function cacheImages(msg: ChatMessage): void {
  if (msg.images && msg.images.length > 0) {
    _cache.set(cacheKey(msg.from || msg.role || '', msg.text), msg.images);
  }
}

/** Restore images from the cache onto a message if it's missing them. */
export function restoreImages(msg: ChatMessage): ChatMessage {
  if (msg.images && msg.images.length > 0) return msg;
  const cached = _cache.get(cacheKey(msg.from || msg.role || '', msg.text));
  return cached ? { ...msg, images: cached } : msg;
}

/** Clear the entire image cache. */
export function clearImageCache(): void {
  _cache.clear();
}
