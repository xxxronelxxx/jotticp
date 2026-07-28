/**
 * JottiCP — shared utility helpers.
 * Performance-oriented: debounce, throttle, formatting.
 */

// ── Timing ────────────────────────────────────────────────────────────────────

/**
 * Returns a debounced version of `fn` that delays invocation until
 * `delay` ms have elapsed since the last call.  Safe to use with
 * Svelte on:input handlers.
 */
export function debounce<T extends (...args: unknown[]) => unknown>(fn: T, delay: number): T {
  let timer: ReturnType<typeof setTimeout>;
  return ((...args: unknown[]) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), delay);
  }) as T;
}

/**
 * Returns a throttled version of `fn` that can fire at most once per
 * `limit` ms.  Useful for scroll/resize handlers.
 */
export function throttle<T extends (...args: unknown[]) => unknown>(fn: T, limit: number): T {
  let inThrottle = false;
  return ((...args: unknown[]) => {
    if (!inThrottle) {
      fn(...args);
      inThrottle = true;
      setTimeout(() => { inThrottle = false; }, limit);
    }
  }) as T;
}

// ── Date / time ───────────────────────────────────────────────────────────────

/** Human-readable relative time: "just now", "5m ago", "3h ago", "2d ago". */
export function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const s = Math.floor(diff / 1000);
  if (s < 60)     return 'just now';
  if (s < 3600)   return `${Math.floor(s / 60)}m ago`;
  if (s < 86400)  return `${Math.floor(s / 3600)}h ago`;
  if (s < 604800) return `${Math.floor(s / 86400)}d ago`;
  return new Date(dateStr).toLocaleDateString();
}

// ── Byte / number formatting ──────────────────────────────────────────────────

/** Format raw bytes to a human-readable string: "1.4 MB", "3.20 GB", etc. */
export function formatBytes(bytes: number): string {
  if (bytes < 1024)       return `${bytes} B`;
  if (bytes < 1_048_576)  return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1_073_741_824) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
}

/** Locale-aware number formatting: 1_234_567 → "1,234,567". */
export function formatNumber(n: number): string {
  return n.toLocaleString();
}
