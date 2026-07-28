<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import type { Site } from '$api/client';
  import { auth } from '$lib/stores/auth';

  // ── Props ─────────────────────────────────────────────────────────────────────

  export let siteId: string;
  export let site: Site;

  // ── Types ─────────────────────────────────────────────────────────────────────

  interface OpcacheStats {
    hit_rate: number;
    memory_used_mb: number;
    memory_total_mb: number;
    cached_scripts: number;
  }

  interface ValkeyStats {
    memory_used_mb: number;
    hit_rate: number;
    connected_clients: number;
    keys: number;
    uptime_seconds: number;
  }

  type CachePreset = 'aggressive' | 'balanced' | 'minimal' | 'disabled';

  interface Toast { message: string; type: 'success' | 'error' }

  // ── Preset definitions ────────────────────────────────────────────────────────

  const PRESETS: { id: CachePreset; label: string; description: string; header: string }[] = [
    {
      id: 'aggressive',
      label: 'Aggressive',
      description: 'Cache for 1 year, CDN-friendly',
      header: 'max-age=31536000, public, immutable',
    },
    {
      id: 'balanced',
      label: 'Balanced',
      description: 'Cache for 1 week, revalidate',
      header: 'max-age=604800, must-revalidate',
    },
    {
      id: 'minimal',
      label: 'Minimal',
      description: 'Cache for 1 hour',
      header: 'max-age=3600',
    },
    {
      id: 'disabled',
      label: 'Disabled',
      description: 'No caching',
      header: 'no-store, no-cache',
    },
  ];

  // ── State ─────────────────────────────────────────────────────────────────────

  let opcache: OpcacheStats | null = null;
  let opcacheLoading = true;
  let opcacheError = false;
  let opcacheFlushing = false;
  let opcacheFlushed = false;

  let valkey: ValkeyStats | null = null;
  let valkeyLoading = true;
  let valkeyError = false;
  let valkeyFlushing = false;
  let valkeyFlushed = false;

  let selectedPreset: CachePreset | null = null;
  let applyingPreset = false;

  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Auth helper ───────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const t = get(auth).token;
    return { 'Content-Type': 'application/json', ...(t ? { Authorization: 'Bearer ' + t } : {}) };
  }

  // ── Toast helper ──────────────────────────────────────────────────────────────

  function showToast(message: string, type: 'success' | 'error' = 'success') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 4000);
  }

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function hitRateColor(rate: number): string {
    if (rate >= 80) return 'bg-green-500';
    if (rate >= 50) return 'bg-yellow-500';
    return 'bg-red-500';
  }

  function hitRateTextColor(rate: number): string {
    if (rate >= 80) return 'text-green-400';
    if (rate >= 50) return 'text-yellow-400';
    return 'text-red-400';
  }

  function formatUptime(seconds: number): string {
    if (seconds < 60) return `${seconds}s`;
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    if (days > 0) return `${days} day${days !== 1 ? 's' : ''} ${hours} hr${hours !== 1 ? 's' : ''}`;
    return `${hours} hr${hours !== 1 ? 's' : ''}`;
  }

  $: opcacheMemPct = opcache
    ? Math.round((opcache.memory_used_mb / Math.max(opcache.memory_total_mb, 1)) * 100)
    : 0;

  // ── Data loading ──────────────────────────────────────────────────────────────

  onMount(async () => {
    await Promise.all([loadOpcache(), loadValkey()]);
  });

  onDestroy(() => { if (toastTimer) clearTimeout(toastTimer); });

  async function loadOpcache() {
    opcacheLoading = true;
    opcacheError = false;
    try {
      const res = await fetch(`/api/v1/cache/opcache/${siteId}`, { headers: authH() });
      if (!res.ok) throw new Error('failed');
      opcache = await res.json() as OpcacheStats;
    } catch {
      opcacheError = true;
    } finally {
      opcacheLoading = false;
    }
  }

  async function loadValkey() {
    valkeyLoading = true;
    valkeyError = false;
    try {
      const res = await fetch(`/api/v1/cache/valkey/${siteId}`, { headers: authH() });
      if (!res.ok) throw new Error('failed');
      valkey = await res.json() as ValkeyStats;
    } catch {
      valkeyError = true;
    } finally {
      valkeyLoading = false;
    }
  }

  // ── Actions ───────────────────────────────────────────────────────────────────

  async function flushOpcache() {
    opcacheFlushing = true;
    opcacheFlushed = false;
    try {
      const res = await fetch(`/api/v1/cache/opcache/${siteId}/flush`, {
        method: 'POST',
        headers: authH(),
        body: '{}',
      });
      if (!res.ok) throw new Error('failed');
      opcacheFlushed = true;
      setTimeout(() => { opcacheFlushed = false; }, 2500);
      await loadOpcache();
    } catch {
      showToast('Failed to flush OPcache', 'error');
    } finally {
      opcacheFlushing = false;
    }
  }

  async function flushValkey() {
    valkeyFlushing = true;
    valkeyFlushed = false;
    try {
      const res = await fetch(`/api/v1/cache/valkey/${siteId}/flush`, {
        method: 'POST',
        headers: authH(),
        body: '{}',
      });
      if (!res.ok) throw new Error('failed');
      valkeyFlushed = true;
      setTimeout(() => { valkeyFlushed = false; }, 2500);
      await loadValkey();
    } catch {
      showToast('Failed to flush Valkey cache', 'error');
    } finally {
      valkeyFlushing = false;
    }
  }

  async function applyPreset() {
    if (!selectedPreset) return;
    applyingPreset = true;
    try {
      const res = await fetch(`/api/v1/cache/headers/${siteId}`, {
        method: 'PUT',
        headers: authH(),
        body: JSON.stringify({ preset: selectedPreset }),
      });
      if (!res.ok) throw new Error('failed');
      const preset = PRESETS.find(p => p.id === selectedPreset);
      showToast(`Cache headers set to "${preset?.label ?? selectedPreset}"`, 'success');
    } catch {
      showToast('Failed to apply cache headers preset', 'error');
    } finally {
      applyingPreset = false;
    }
  }

  function warmCache() {
    window.open(`https://${site.domain}`, '_blank', 'noopener,noreferrer');
  }
</script>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  .card-hover { transition: all 0.2s ease; }
  .card-hover:hover { transform: translateY(-1px); box-shadow: 0 8px 30px rgba(0,0,0,0.2); }
</style>

<!-- ── Content ──────────────────────────────────────────────────────────────── -->
<div class="tab-content space-y-5">

  <!-- ── OPcache + Valkey side by side ─────────────────────────────────────── -->
  <div class="grid grid-cols-1 lg:grid-cols-2 gap-5">

    <!-- OPcache card -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="flex items-center justify-between px-5 py-4 border-b border-border">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M3.75 13.5l10.5-11.25L12 10.5h8.25L9.75 21.75 12 13.5H3.75z"/>
            </svg>
          </div>
          <h3 class="text-sm font-semibold text-foreground">OPcache</h3>
        </div>
        {#if !opcacheLoading && !opcacheError}
          <button
            on:click={flushOpcache}
            disabled={opcacheFlushing}
            class="h-7 px-3 rounded-lg text-xs font-medium transition-colors
                   disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center gap-1.5
                   {opcacheFlushed
                     ? 'bg-green-500/10 text-green-400 border border-green-500/20'
                     : 'text-orange-400 hover:bg-orange-500/10'}"
          >
            {#if opcacheFlushing}
              <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
              Flushing...
            {:else if opcacheFlushed}
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
              </svg>
              Flushed!
            {:else}
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Flush OPcache
            {/if}
          </button>
        {/if}
      </div>

      {#if opcacheLoading}
        <div class="p-5 space-y-4 animate-pulse">
          <div class="h-8 bg-muted rounded w-20"></div>
          <div class="h-2 bg-muted rounded w-full"></div>
          <div class="grid grid-cols-2 gap-3">
            {#each [0, 1] as _}
              <div class="space-y-1.5">
                <div class="h-3 bg-muted rounded w-12"></div>
                <div class="h-5 bg-muted rounded w-16"></div>
              </div>
            {/each}
          </div>
        </div>
      {:else if opcacheError}
        <div class="p-6 text-center">
          <p class="text-sm text-muted-foreground mb-3">Failed to load OPcache stats</p>
          <button
            on:click={loadOpcache}
            class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors"
          >
            Retry
          </button>
        </div>
      {:else if opcache}
        {@const opcacheCircumference = 2 * Math.PI * 18}
        {@const opcacheDashOffset = opcacheCircumference - (Math.min(opcache.hit_rate, 100) / 100) * opcacheCircumference}
        <div class="p-5 space-y-4">
          <!-- Hit rate donut -->
          <div class="flex items-center gap-4">
            <div class="relative w-16 h-16 shrink-0">
              <svg class="w-16 h-16 -rotate-90" viewBox="0 0 44 44">
                <circle cx="22" cy="22" r="18" fill="none" stroke="currentColor" class="text-muted/40" stroke-width="4"/>
                <circle cx="22" cy="22" r="18" fill="none"
                  stroke="{opcache.hit_rate >= 80 ? '#4ade80' : opcache.hit_rate >= 50 ? '#fbbf24' : '#f87171'}"
                  stroke-width="4" stroke-linecap="round"
                  stroke-dasharray="{opcacheCircumference}"
                  stroke-dashoffset="{opcacheDashOffset}"
                  class="transition-all duration-700"/>
              </svg>
              <div class="absolute inset-0 flex flex-col items-center justify-center">
                <span class="text-[11px] font-bold {hitRateTextColor(opcache.hit_rate)} leading-none">{opcache.hit_rate.toFixed(0)}%</span>
              </div>
            </div>
            <div class="flex-1">
              <p class="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-0.5">Hit Rate</p>
              <p class="text-xl font-bold {hitRateTextColor(opcache.hit_rate)}">{opcache.hit_rate.toFixed(1)}%</p>
            </div>
          </div>

          <!-- Memory -->
          <div>
            <div class="flex items-center justify-between mb-1.5">
              <span class="text-xs font-medium text-muted-foreground uppercase tracking-wider">Memory</span>
              <span class="text-xs text-muted-foreground">
                {opcache.memory_used_mb.toFixed(0)} / {opcache.memory_total_mb.toFixed(0)} MB
              </span>
            </div>
            <div class="h-2 bg-muted rounded-full overflow-hidden">
              <div
                class="h-full rounded-full bg-blue-500 transition-all"
                style="width: {Math.min(opcacheMemPct, 100)}%"
              ></div>
            </div>
            <p class="text-xs text-muted-foreground mt-1">{opcacheMemPct}% used</p>
          </div>

          <!-- Cached scripts -->
          <div class="flex items-center justify-between py-2 border-t border-border">
            <span class="text-xs text-muted-foreground">Cached Scripts</span>
            <span class="text-sm font-semibold text-foreground">{opcache.cached_scripts.toLocaleString()}</span>
          </div>
        </div>
      {/if}
    </div>

    <!-- Valkey card -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="flex items-center justify-between px-5 py-4 border-b border-border">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-red-500/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/>
            </svg>
          </div>
          <h3 class="text-sm font-semibold text-foreground">Valkey Cache</h3>
        </div>
        {#if !valkeyLoading && !valkeyError}
          <button
            on:click={flushValkey}
            disabled={valkeyFlushing}
            class="h-7 px-3 rounded-lg text-xs font-medium transition-colors
                   disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center gap-1.5
                   {valkeyFlushed
                     ? 'bg-green-500/10 text-green-400 border border-green-500/20'
                     : 'text-orange-400 hover:bg-orange-500/10'}"
          >
            {#if valkeyFlushing}
              <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
              Flushing...
            {:else if valkeyFlushed}
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
              </svg>
              Flushed!
            {:else}
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Flush Valkey
            {/if}
          </button>
        {/if}
      </div>

      {#if valkeyLoading}
        <div class="p-5 space-y-3 animate-pulse">
          {#each [0, 1, 2, 3] as _}
            <div class="flex items-center justify-between">
              <div class="h-3 bg-muted rounded w-20"></div>
              <div class="h-4 bg-muted rounded w-16"></div>
            </div>
          {/each}
        </div>
      {:else if valkeyError}
        <div class="p-6 text-center">
          <p class="text-sm text-muted-foreground mb-3">Failed to load Valkey stats</p>
          <button
            on:click={loadValkey}
            class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors"
          >
            Retry
          </button>
        </div>
      {:else if valkey}
        {@const valkeyCircumference = 2 * Math.PI * 18}
        {@const valkeyDashOffset = valkeyCircumference - (Math.min(valkey.hit_rate, 100) / 100) * valkeyCircumference}
        <div class="p-5 divide-y divide-border">
          <!-- Hit rate donut -->
          <div class="pb-3 mb-3 flex items-center gap-4">
            <div class="relative w-16 h-16 shrink-0">
              <svg class="w-16 h-16 -rotate-90" viewBox="0 0 44 44">
                <circle cx="22" cy="22" r="18" fill="none" stroke="currentColor" class="text-muted/40" stroke-width="4"/>
                <circle cx="22" cy="22" r="18" fill="none"
                  stroke="{valkey.hit_rate >= 80 ? '#4ade80' : valkey.hit_rate >= 50 ? '#fbbf24' : '#f87171'}"
                  stroke-width="4" stroke-linecap="round"
                  stroke-dasharray="{valkeyCircumference}"
                  stroke-dashoffset="{valkeyDashOffset}"
                  class="transition-all duration-700"/>
              </svg>
              <div class="absolute inset-0 flex items-center justify-center">
                <span class="text-[11px] font-bold {hitRateTextColor(valkey.hit_rate)} leading-none">{valkey.hit_rate.toFixed(0)}%</span>
              </div>
            </div>
            <div class="flex-1">
              <p class="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-0.5">Hit Rate</p>
              <p class="text-xl font-bold {hitRateTextColor(valkey.hit_rate)}">{valkey.hit_rate.toFixed(1)}%</p>
            </div>
          </div>

          <!-- Stats list -->
          {#each [
            { label: 'Memory Used', value: `${valkey.memory_used_mb.toFixed(1)} MB` },
            { label: 'Total Keys', value: valkey.keys.toLocaleString() },
            { label: 'Connected Clients', value: String(valkey.connected_clients) },
            { label: 'Uptime', value: formatUptime(valkey.uptime_seconds) },
          ] as stat}
            <div class="flex items-center justify-between py-2">
              <span class="text-xs text-muted-foreground">{stat.label}</span>
              <span class="text-sm font-medium text-foreground">{stat.value}</span>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>

  <!-- ── Browser cache headers presets ─────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden">
    <div class="flex items-center justify-between px-5 py-4 border-b border-border">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 21a9.004 9.004 0 008.716-6.747M12 21a9.004 9.004 0 01-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 017.843 4.582M12 3a8.997 8.997 0 00-7.843 4.582"/>
          </svg>
        </div>
        <div>
          <h3 class="text-sm font-semibold text-foreground">Browser Cache Headers</h3>
          <p class="text-xs text-muted-foreground mt-0.5">Set Cache-Control headers for static assets</p>
        </div>
      </div>
      <button
        on:click={applyPreset}
        disabled={!selectedPreset || applyingPreset}
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
               hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed
               inline-flex items-center gap-2"
      >
        {#if applyingPreset}
          <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
          </svg>
          Applying...
        {:else}
          Apply
        {/if}
      </button>
    </div>

    <div class="p-5 grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
      {#each PRESETS as preset}
        {@const isSelected = selectedPreset === preset.id}
        <button
          on:click={() => selectedPreset = preset.id}
          class="relative text-left rounded-xl border p-4 transition-all
            {isSelected
              ? 'border-primary bg-primary/5 ring-2 ring-primary shadow-md shadow-primary/10'
              : 'border-border hover:border-muted-foreground/30 hover:bg-muted/50'}"
        >
          {#if isSelected}
            <span class="absolute top-2.5 right-2.5 flex items-center justify-center w-5 h-5 rounded-full bg-primary shadow-sm">
              <svg class="w-3 h-3 text-primary-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3">
                <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
              </svg>
            </span>
          {/if}
          <p class="text-sm font-semibold text-foreground mb-1 pr-6">{preset.label}</p>
          <p class="text-xs text-muted-foreground mb-2">{preset.description}</p>
          <p class="text-xs font-mono text-muted-foreground/70 truncate">{preset.header}</p>
        </button>
      {/each}
    </div>
  </div>

  <!-- ── Cache warming ──────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-5">
    <div class="flex items-start gap-4">
      <div class="w-10 h-10 rounded-full bg-muted flex items-center justify-center shrink-0">
        <svg class="w-5 h-5 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M17.657 18.657A8 8 0 016.343 7.343S7 9 9 10c0-2 .5-5 2.986-7C14 5 16.09 5.777 17.656 7.343A7.975 7.975 0 0120 13a7.975 7.975 0 01-2.343 5.657z" />
          <path stroke-linecap="round" stroke-linejoin="round" d="M9.879 16.121A3 3 0 1012.015 11L11 14H9c0 .768.293 1.536.879 2.121z" />
        </svg>
      </div>
      <div class="flex-1 min-w-0">
        <h3 class="text-sm font-semibold text-foreground mb-0.5">Cache Warming</h3>
        <p class="text-xs text-muted-foreground">
          Visit your site to warm the cache after flushing. This triggers the first page load
          so subsequent visitors get cached responses.
        </p>
      </div>
      <button
        on:click={warmCache}
        class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
               hover:bg-muted transition-colors inline-flex items-center gap-2 shrink-0"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
        </svg>
        Warm Cache Now
      </button>
    </div>
  </div>

</div>

<!-- ── Toast ──────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div class="fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border transition-all
    {toast.type === 'success' ? 'bg-green-950/90 border-green-800 text-green-300' : 'bg-red-950/90 border-red-800 text-red-300'}"
    role="alert" aria-live="polite">
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
    <button on:click={() => toast = null} class="ml-2 opacity-60 hover:opacity-100 transition-opacity" aria-label="Dismiss">
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>
{/if}
