<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import type { Site } from '$api/client';
  import { api } from '$api/client';

  // ── Props ─────────────────────────────────────────────────────────────────────

  export let siteId: string;
  export let site: Site;

  // ── State ─────────────────────────────────────────────────────────────────────

  let logType: 'access' | 'error' | 'php' = 'access';
  let lineCount = 200;
  let searchQuery = '';
  let autoRefresh = false;
  let logLoading = true;
  let logLines: string[] = [];
  let logContainer: HTMLDivElement;
  let refreshInterval: ReturnType<typeof setInterval> | null = null;

  // toast
  interface Toast { message: string; type: 'success' | 'error' }
  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function showToast(message: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 4000);
  }

  function logPath(type: string): string {
    const d = site.domain;
    if (type === 'access') return `/var/log/nginx/${d}-access.log`;
    if (type === 'error') return `/var/log/nginx/${d}-error.log`;
    return '/var/log/php8.3-fpm.log';
  }

  function lineColor(line: string): string {
    if (line.includes('error') || line.includes('ERROR') || line.includes(' 5')) return 'text-red-400';
    if (line.includes('warn') || line.includes('WARN') || line.includes(' 4')) return 'text-yellow-400';
    if (line.includes(' 200 ') || line.includes(' 201 ')) return 'text-green-400';
    return 'text-slate-300';
  }

  $: filteredLines = searchQuery.trim()
    ? logLines.filter(l => l.toLowerCase().includes(searchQuery.toLowerCase()))
    : logLines;

  // Auto-refresh: start/stop interval when toggle changes.
  // Using a function avoids the reactive block re-running on unrelated state changes.
  function setAutoRefresh(enabled: boolean) {
    if (refreshInterval) {
      clearInterval(refreshInterval);
      refreshInterval = null;
    }
    if (enabled) {
      refreshInterval = setInterval(() => { void loadLogs(); }, 5000);
    }
  }
  $: setAutoRefresh(autoRefresh);

  // ── Data loading ──────────────────────────────────────────────────────────────

  onMount(loadLogs);
  onDestroy(() => {
    if (refreshInterval) clearInterval(refreshInterval);
    if (toastTimer) clearTimeout(toastTimer);
  });

  // Sequence counter to drop stale responses when multiple loads race
  let loadSeq = 0;

  async function loadLogs() {
    const seq = ++loadSeq;
    logLoading = true;
    try {
      const data = await api.sites.logs(siteId, logType, lineCount);
      if (seq !== loadSeq) return;
      const content = data.content ?? '';
      logLines = content ? content.split('\n').filter(Boolean) : [];
    } catch {
      if (seq !== loadSeq) return;
      logLines = [];
    } finally {
      if (seq === loadSeq) logLoading = false;
    }
  }

  async function switchLogType(type: 'access' | 'error' | 'php') {
    logType = type;
    await loadLogs();
  }

  async function changeLineCount(val: number) {
    lineCount = val;
    await loadLogs();
  }

  function scrollToBottom() {
    if (logContainer) {
      logContainer.scrollTop = logContainer.scrollHeight;
    }
  }

  function downloadLog() {
    const path = logPath(logType);
    window.location.href = `/api/v1/files/read?path=${encodeURIComponent(path)}&download=1`;
  }

  const logTabs: { key: 'access' | 'error' | 'php'; label: string }[] = [
    { key: 'access', label: 'Access Log' },
    { key: 'error', label: 'Error Log' },
    { key: 'php', label: 'PHP Log' },
  ];
</script>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  .card-hover { transition: all 0.2s ease; }
  .card-hover:hover { transform: translateY(-1px); box-shadow: 0 8px 30px rgba(0,0,0,0.2); }
</style>

<div class="tab-content space-y-4">

  <!-- ── Log type tabs with icons ────────────────────────────────────────────── -->
  <div class="flex items-center gap-1 bg-muted/40 rounded-xl p-1 w-fit">
    {#each logTabs as tab}
      <button
        on:click={() => switchLogType(tab.key)}
        class="h-8 px-4 rounded-lg text-sm font-medium transition-colors inline-flex items-center gap-1.5
          {logType === tab.key
            ? 'bg-card border border-border text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground'}"
      >
        {#if tab.key === 'access'}
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 21a9.004 9.004 0 008.716-6.747M12 21a9.004 9.004 0 01-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3"/>
          </svg>
        {:else if tab.key === 'error'}
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z"/>
          </svg>
        {:else}
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M14.25 9.75L16.5 12l-2.25 2.25m-4.5 0L7.5 12l2.25-2.25M6 20.25h12A2.25 2.25 0 0020.25 18V6A2.25 2.25 0 0018 3.75H6A2.25 2.25 0 003.75 6v12A2.25 2.25 0 006 20.25z"/>
          </svg>
        {/if}
        {tab.label}
      </button>
    {/each}
  </div>

  <!-- ── Controls row ───────────────────────────────────────────────────────── -->
  <div class="flex flex-wrap items-center gap-2">
    <!-- Lines selector -->
    <select
      value={lineCount}
      on:change={(e) => changeLineCount(parseInt((e.target as HTMLSelectElement).value))}
      class="h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
    >
      <option value={50}>50 lines</option>
      <option value={100}>100 lines</option>
      <option value={200}>200 lines</option>
      <option value={500}>500 lines</option>
    </select>

    <!-- Search -->
    <div class="relative flex-1 min-w-[180px] max-w-xs">
      <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground pointer-events-none" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-5.197-5.197m0 0A7.5 7.5 0 105.196 15.803a7.5 7.5 0 0010.607 10.607z" />
      </svg>
      <input
        bind:value={searchQuery}
        placeholder="Filter logs..."
        class="w-full h-9 pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
      />
      {#if searchQuery}
        <button
          on:click={() => { searchQuery = ''; }}
          class="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors"
          aria-label="Clear search"
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      {/if}
    </div>

    <!-- Auto-refresh toggle with live indicator -->
    <label class="flex items-center gap-2 cursor-pointer select-none">
      <button
        on:click={() => { autoRefresh = !autoRefresh; }}
        role="switch"
        aria-checked={autoRefresh}
        class="relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors focus:outline-none focus:ring-2 focus:ring-ring
          {autoRefresh ? 'bg-primary' : 'bg-muted'}"
      >
        <span class="pointer-events-none inline-block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform
          {autoRefresh ? 'translate-x-4' : 'translate-x-0'}"></span>
      </button>
      <div class="flex items-center gap-1.5">
        {#if autoRefresh}
          <span class="w-2 h-2 rounded-full bg-green-400 animate-pulse"></span>
          <span class="text-xs font-medium text-green-400 whitespace-nowrap">Live</span>
        {:else}
          <span class="text-xs font-medium text-muted-foreground whitespace-nowrap">Auto 5s</span>
        {/if}
      </div>
    </label>

    <!-- Spacer -->
    <div class="flex-1"></div>

    <!-- Download with cloud-download icon -->
    <button
      on:click={downloadLog}
      title="Download log"
      class="h-9 px-3 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted hover:text-foreground transition-all active:scale-95 inline-flex items-center gap-2"
    >
      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M12 10v6m0 0l-3-3m3 3l3-3m2 8H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
      </svg>
      Download
    </button>

    <!-- Refresh -->
    <button
      on:click={loadLogs}
      disabled={logLoading}
      title="Refresh"
      class="h-9 w-9 rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center justify-center"
    >
      <svg class="w-3.5 h-3.5 {logLoading ? 'animate-spin' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0l3.181 3.183a8.25 8.25 0 0013.803-3.7M4.031 9.865a8.25 8.25 0 0113.803-3.7l3.181 3.182m0-4.991v4.99" />
      </svg>
    </button>
  </div>

  <!-- ── Log viewer ─────────────────────────────────────────────────────────── -->
  <div class="bg-slate-950 border border-border rounded-xl overflow-hidden">
    <!-- Toolbar -->
    <div class="flex items-center justify-between px-4 py-2 border-b border-border/40 bg-muted/40">
      <div class="flex items-center gap-3">
        <span class="text-xs text-muted-foreground font-mono">
          {#if logLoading}
            Loading…
          {:else if searchQuery && filteredLines.length !== logLines.length}
            {filteredLines.length} of {logLines.length} lines (filtered)
          {:else}
            {filteredLines.length} lines
          {/if}
        </span>
        {#if autoRefresh}
          <span class="inline-flex items-center gap-1 text-[10px] font-medium text-green-400">
            <span class="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse"></span>
            Live
          </span>
        {/if}
      </div>
      <button
        on:click={scrollToBottom}
        class="text-xs text-muted-foreground hover:text-foreground transition-colors font-mono inline-flex items-center gap-1"
      >
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M19.5 13.5L12 21m0 0l-7.5-7.5M12 21V3" />
        </svg>
        Bottom
      </button>
    </div>

    <!-- Log content -->
    {#if logLoading}
      <!-- Skeleton loader -->
      <div class="p-4 space-y-2">
        {#each Array(10) as _}
          <div class="flex gap-3">
            <div class="w-8 h-4 bg-muted/60 rounded animate-pulse shrink-0"></div>
            <div class="h-4 bg-muted/60 rounded animate-pulse" style="width: {40 + Math.random() * 50}%"></div>
          </div>
        {/each}
      </div>

    {:else if filteredLines.length === 0}
      <!-- Empty state -->
      <div class="flex flex-col items-center justify-center py-16 px-4 text-center">
        <div class="w-12 h-12 mb-3 rounded-full bg-muted flex items-center justify-center">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M11.25 11.25l.041-.02a.75.75 0 011.063.852l-.708 2.836a.75.75 0 001.063.853l.041-.021M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-9-3.75h.008v.008H12V8.25z" />
          </svg>
        </div>
        {#if searchQuery}
          <p class="text-sm font-medium text-muted-foreground mb-1">No matching lines</p>
          <p class="text-xs text-muted-foreground/70">No log lines contain "{searchQuery}"</p>
        {:else}
          <p class="text-sm font-medium text-muted-foreground mb-1">Log file is empty or not found</p>
          <p class="text-xs text-muted-foreground/70">{logPath(logType)}</p>
        {/if}
      </div>

    {:else}
      <!-- Lines -->
      <div bind:this={logContainer} class="overflow-y-auto max-h-[600px] p-4 font-mono text-xs leading-relaxed">
        {#each filteredLines as line, i}
          <div class="group flex gap-3 hover:bg-muted/20 px-1 rounded">
            <span class="text-slate-600 group-hover:text-slate-500 select-none w-8 text-right shrink-0 tabular-nums transition-colors">{i + 1}</span>
            <span class="{lineColor(line)} break-all whitespace-pre-wrap">{line}</span>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <!-- Log path hint -->
  {#if !logLoading}
    <p class="text-[10px] text-muted-foreground font-mono px-1">{logPath(logType)}</p>
  {/if}
</div>

<!-- ── Toast ──────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div
    class="fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border transition-all duration-300
      {toast.type === 'success' ? 'bg-green-950/90 border-green-800 text-green-300' : 'bg-red-950/90 border-red-800 text-red-300'}"
    role="alert"
    aria-live="polite"
  >
    {#if toast.type === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
      </svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
      </svg>
    {/if}
    <span class="text-sm font-medium">{toast.message}</span>
    <button on:click={() => { toast = null; }} class="text-current opacity-60 hover:opacity-100 transition-opacity" aria-label="Dismiss">
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>
{/if}
