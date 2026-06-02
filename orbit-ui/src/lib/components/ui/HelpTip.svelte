<script lang="ts">
  export let title: string;
  export let content: string; // markdown string

  let open = false;

  // Simple markdown-to-HTML converter (handles ## headers, **bold**, - lists, `code`, paragraphs)
  function markdownToHtml(md: string): string {
    return md
      // Headers
      .replace(/^## (.+)$/gm, '<h2>$1</h2>')
      // Bold
      .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
      // Inline code
      .replace(/`([^`]+)`/g, '<code>$1</code>')
      // List items
      .replace(/^- (.+)$/gm, '<li>$1</li>')
      // Wrap consecutive <li> in <ul>
      .replace(/(<li>.*?<\/li>\n?)+/gs, (match) => `<ul>${match}</ul>`)
      // Paragraphs (non-tag lines)
      .replace(/^(?!<[hul]|$)(.+)$/gm, '<p>$1</p>')
      // Clean up empty lines
      .replace(/\n{2,}/g, '')
      .trim();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') open = false;
  }
</script>

<svelte:window on:keydown={handleKeydown} />

<!-- "?" trigger button -->
<button
  type="button"
  on:click={() => open = !open}
  aria-label="Help: {title}"
  aria-expanded={open}
  class="inline-flex items-center justify-center w-5 h-5 rounded-full text-xs font-semibold
         border border-[var(--text-subtle)] text-[var(--text-subtle)]
         hover:border-[var(--accent)] hover:text-[var(--accent)] hover:bg-[var(--accent-light)]
         transition-colors focus:outline-none focus:ring-2 focus:ring-[var(--ring)] focus:ring-offset-1
         shrink-0"
>
  ?
</button>

<!-- Backdrop overlay -->
{#if open}
  <div
    class="fixed inset-0 z-40 bg-black/20 backdrop-blur-[1px]"
    on:click={() => open = false}
    role="presentation"
    aria-hidden="true"
  ></div>
{/if}

<!-- Slide-in panel (360px wide, from right) -->
<div
  class="fixed top-0 right-0 bottom-0 z-50 w-[360px] bg-[var(--bg-page)] border-l border-[var(--border)]
         shadow-2xl flex flex-col transition-transform duration-300 ease-in-out"
  class:translate-x-0={open}
  class:translate-x-full={!open}
  role="dialog"
  aria-modal="true"
  aria-labelledby="helptip-title"
>
  <!-- Panel header -->
  <div class="flex items-center justify-between px-5 py-4 border-b border-[var(--border)] shrink-0">
    <h2 id="helptip-title" class="text-base font-semibold text-[var(--text-primary)] flex items-center gap-2"><svg class="w-4 h-4 text-primary shrink-0" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>
      {title}
    </h2>
    <button
      type="button"
      on:click={() => open = false}
      aria-label="Close help panel"
      class="w-8 h-8 inline-flex items-center justify-center rounded-lg
             text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-[var(--bg-hover)]
             transition-colors focus:outline-none focus:ring-2 focus:ring-[var(--ring)]"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>

  <!-- Panel content (scrollable) -->
  <div class="flex-1 overflow-y-auto px-5 py-4">
    <div class="prose text-sm">
      <!-- eslint-disable-next-line svelte/no-at-html-tags -->
      {@html markdownToHtml(content)}
    </div>
  </div>

  <!-- Panel footer -->
  <div class="shrink-0 px-5 py-3 border-t border-[var(--border)]">
    <a
      href="https://docs.orbitcp.io"
      target="_blank"
      rel="noopener noreferrer"
      class="text-xs text-[var(--accent)] hover:underline inline-flex items-center gap-1"
    >
      Full documentation
      <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
          d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
      </svg>
    </a>
  </div>
</div>
