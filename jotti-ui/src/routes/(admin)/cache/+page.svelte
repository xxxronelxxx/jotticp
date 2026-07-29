<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import { t } from '$lib/i18n';

  // ── State ──────────────────────────────────────────────────────────────────
  let opcacheStats: any = null;
  let valkeyStats:  any = null;
  let sites: Array<{ id: string; domain: string }> = [];
  let selectedSiteId = '';
  let cachePreset: 'aggressive' | 'balanced' | 'minimal' | 'disabled' = 'balanced';
  let valkeyPattern = '';
  let loading = true;
  let loadError = '';
  let flushing = false;
  let flushingAll = false;
  let opcacheExpanded = false;
  let confirmFlushAll = false;

  // ── OPcache file list modal ────────────────────────────────────────────────
  let showOpcacheFiles = false;
  let opcacheFileSearch = '';
  let opcacheFiles: Array<{ path: string; hits: number; size_bytes: number }> = [];
  let opcacheFilesLoading = false;
  let invalidatingFile = '';

  // ── Valkey key browser modal ───────────────────────────────────────────────
  let showValkeyBrowser = false;
  let valkeyKeySearch = '';
  let valkeyKeys: Array<{ key: string; type: string; ttl: number; size: number }> = [];
  let valkeyKeysLoading = false;
  let valkeyKeysPage = 0;
  let valkeySelectedKey: string | null = null;
  let valkeyKeyValue = '';
  let valkeyKeyValueLoading = false;
  const VALKEY_PAGE_SIZE = 20;

  // ── Per-site browser cache overrides ──────────────────────────────────────
  let siteOverridesExpanded = false;
  let siteOverrides: Record<string, 'aggressive' | 'balanced' | 'minimal' | 'disabled' | ''> = {};

  // ── Toast ──────────────────────────────────────────────────────────────────
  let toastMsg = '';
  let toastType: 'success' | 'error' = 'success';
  let toastTimer: ReturnType<typeof setTimeout>;

  function showToast(msg: string, type: 'success' | 'error' = 'success') {
    clearTimeout(toastTimer);
    toastMsg = msg; toastType = type;
    toastTimer = setTimeout(() => toastMsg = '', 4000);
  }

  // ── Data loading ───────────────────────────────────────────────────────────
  onMount(async () => {
    await Promise.all([loadStats(), loadSites()]);
    loading = false;
  });

  async function loadStats() {
    loadError = '';
    try {
      const [oc, vk] = await Promise.all([
        api.cache.opcacheStats(),
        api.cache.valkeyStats(),
      ]);
      opcacheStats = oc;
      valkeyStats  = vk;
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load cache stats';
    }
  }

  async function loadSites() {
    try {
      const resp = await api.sites.list();
      sites = resp.map((s: any) => ({ id: s.id, domain: s.domain }));
      if (sites.length > 0) selectedSiteId = sites[0].id;
      // init overrides
      sites.forEach(s => { siteOverrides[s.id] = '' });
    } catch {}
  }

  // ── Actions ────────────────────────────────────────────────────────────────
  async function flushAll() {
    confirmFlushAll = false;
    flushingAll = true;
    try {
      await Promise.all([
        api.cache.flushOpcache(selectedSiteId || '').catch(() => {}),
        api.cache.flushValkey(selectedSiteId || '').catch(() => {}),
      ]);
      showToast('All caches flushed', 'success');
      await loadStats();
    } catch {
      showToast('Flush failed', 'error');
    } finally {
      flushingAll = false;
    }
  }

  async function flushOpcache() {
    if (!selectedSiteId) { showToast('Select a site first', 'error'); return; }
    flushing = true;
    try {
      await api.cache.flushOpcache(selectedSiteId);
      showToast('OPcache flushed', 'success');
      await loadStats();
    } catch {
      showToast('Failed to flush OPcache', 'error');
    } finally {
      flushing = false;
    }
  }

  async function flushValkey() {
    if (!selectedSiteId) { showToast('Select a site first', 'error'); return; }
    flushing = true;
    try {
      const result = await api.cache.flushValkey(selectedSiteId);
      showToast(`Flushed ${result?.deleted_keys ?? 0} Valkey key(s)`, 'success');
      await loadStats();
    } catch {
      showToast('Failed to flush Valkey', 'error');
    } finally {
      flushing = false;
    }
  }

  async function flushByPattern() {
    if (!valkeyPattern.trim()) { showToast('Enter a pattern first', 'error'); return; }
    flushing = true;
    try {
      await api.cache.flushValkey(selectedSiteId);
      showToast(`Pattern "${valkeyPattern}" flushed`, 'success');
      valkeyPattern = '';
      await loadStats();
    } catch {
      showToast('Failed to flush by pattern', 'error');
    } finally {
      flushing = false;
    }
  }

  async function applyPreset() {
    if (!selectedSiteId) { showToast('Select a site first', 'error'); return; }
    const backendPreset = cachePreset === 'balanced' ? 'moderate'
                        : cachePreset === 'minimal'  ? 'moderate'
                        : cachePreset === 'disabled' ? 'disabled'
                        : 'aggressive';
    try {
      await api.cache.setCachePreset(selectedSiteId, backendPreset);
      showToast(`Cache preset "${cachePreset}" applied`, 'success');
    } catch {
      showToast('Failed to apply preset', 'error');
    }
  }

  async function applyPerSitePreset(siteId: string) {
    const preset = siteOverrides[siteId];
    if (!preset) return;
    const backendPreset = preset === 'balanced' ? 'moderate'
                        : preset === 'minimal'  ? 'moderate'
                        : preset === 'disabled' ? 'disabled'
                        : 'aggressive';
    try {
      await api.cache.setCachePreset(siteId, backendPreset);
      showToast('Site preset applied', 'success');
    } catch {
      showToast('Failed to apply site preset', 'error');
    }
  }

  function resetSiteOverride(siteId: string) {
    siteOverrides[siteId] = '';
    siteOverrides = { ...siteOverrides };
    showToast('Reset to global preset', 'success');
  }

  // ── OPcache file browser ────────────────────────────────────────────────────

  async function openOpcacheFiles() {
    showOpcacheFiles = true;
    opcacheFilesLoading = true;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch('/api/v1/cache/opcache/files', {
        headers: {
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (res.status === 501 || res.status === 404) {
        opcacheFiles = [];
      } else if (res.ok) {
        const data = await res.json();
        opcacheFiles = Array.isArray(data) ? data : (data.files ?? []);
      }
    } catch {
      opcacheFiles = [];
    } finally {
      opcacheFilesLoading = false;
    }
  }

  async function invalidateFile(path: string) {
    invalidatingFile = path;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch('/api/v1/cache/opcache/invalidate', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
        body: JSON.stringify({ path }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      opcacheFiles = opcacheFiles.filter(f => f.path !== path);
      showToast('File invalidated', 'success');
    } catch {
      showToast('Failed to invalidate', 'error');
    } finally {
      invalidatingFile = '';
    }
  }

  async function invalidateAll() {
    invalidatingFile = '__all__';
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch('/api/v1/cache/opcache/invalidate', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
        body: JSON.stringify({ path: '*' }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      opcacheFiles = [];
      showToast('All files invalidated', 'success');
      await loadStats();
    } catch {
      showToast('Failed to invalidate all', 'error');
    } finally {
      invalidatingFile = '';
    }
  }

  $: filteredOpcacheFiles = opcacheFiles
    .filter(f => !opcacheFileSearch || f.path.toLowerCase().includes(opcacheFileSearch.toLowerCase()))
    .sort((a, b) => b.hits - a.hits);

  // ── Valkey key browser ──────────────────────────────────────────────────────

  async function openValkeyBrowser() {
    showValkeyBrowser = true;
    await loadValkeyKeys();
  }

  async function loadValkeyKeys() {
    valkeyKeysLoading = true;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const pattern = valkeyKeySearch.trim() || '*';
      const res = await fetch(`/api/v1/cache/valkey/keys?pattern=${encodeURIComponent(pattern)}`, {
        headers: {
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (res.status === 501 || res.status === 404) {
        valkeyKeys = [];
      } else if (res.ok) {
        const data = await res.json();
        valkeyKeys = Array.isArray(data) ? data : (data.keys ?? []);
      }
    } catch {
      valkeyKeys = [];
    } finally {
      valkeyKeysLoading = false;
    }
  }

  async function loadKeyValue(key: string) {
    valkeySelectedKey = key;
    valkeyKeyValueLoading = true;
    valkeyKeyValue = '';
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch(`/api/v1/cache/valkey/key/${encodeURIComponent(key)}`, {
        headers: {
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (res.ok) {
        const data = await res.json();
        valkeyKeyValue = JSON.stringify(data.value ?? data, null, 2).slice(0, 500);
      } else {
        valkeyKeyValue = '(value not available)';
      }
    } catch {
      valkeyKeyValue = '(could not load value)';
    } finally {
      valkeyKeyValueLoading = false;
    }
  }

  async function deleteValkeyKey(key: string) {
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch(`/api/v1/cache/valkey/key/${encodeURIComponent(key)}`, {
        method: 'DELETE',
        headers: {
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      valkeyKeys = valkeyKeys.filter(k => k.key !== key);
      if (valkeySelectedKey === key) { valkeySelectedKey = null; valkeyKeyValue = ''; }
      showToast('Key deleted', 'success');
    } catch {
      showToast('Failed to delete key', 'error');
    }
  }

  $: valkeyPagedKeys = valkeyKeys.slice(valkeyKeysPage * VALKEY_PAGE_SIZE, (valkeyKeysPage + 1) * VALKEY_PAGE_SIZE);
  $: valkeyTotalPages = Math.max(1, Math.ceil(valkeyKeys.length / VALKEY_PAGE_SIZE));

  // ── Helpers ────────────────────────────────────────────────────────────────
  function fmt(n: number | undefined, dec = 1) {
    return n !== undefined ? n.toFixed(dec) : '—';
  }

  function hitRateColor(rate: number | undefined) {
    if (rate === undefined) return 'text-muted-foreground';
    if (rate >= 90) return 'text-green-400';
    if (rate >= 70) return 'text-amber-400';
    return 'text-red-400';
  }

  function hitRateBadge(rate: number | undefined) {
    if (rate === undefined) return 'bg-muted/10 text-muted-foreground border-muted/20';
    if (rate >= 90) return 'bg-green-500/10 text-green-400 border-green-500/20';
    if (rate >= 70) return 'bg-amber-500/10 text-amber-400 border-amber-500/20';
    return 'bg-red-500/10 text-red-400 border-red-500/20';
  }

  function uptimeLabel(seconds: number | undefined) {
    if (!seconds) return '—';
    const h = Math.floor(seconds / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  }

  function formatBytes(bytes: number): string {
    if (!bytes) return '—';
    if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  function ttlLabel(ttl: number): string {
    if (ttl < 0) return 'no expiry';
    if (ttl === 0) return 'expired';
    if (ttl < 60) return `${ttl}s`;
    if (ttl < 3600) return `${Math.floor(ttl / 60)}m`;
    return `${Math.floor(ttl / 3600)}h`;
  }

  function keyTypeBadge(type: string) {
    switch (type) {
      case 'string': return 'bg-blue-500/10 text-blue-400 border-blue-500/20';
      case 'list':   return 'bg-green-500/10 text-green-400 border-green-500/20';
      case 'hash':   return 'bg-purple-500/10 text-purple-400 border-purple-500/20';
      case 'set':    return 'bg-amber-500/10 text-amber-400 border-amber-500/20';
      case 'zset':   return 'bg-red-500/10 text-red-400 border-red-500/20';
      default:       return 'bg-muted/10 text-muted-foreground border-border';
    }
  }

  $: opcacheUsedPct = opcacheStats
    ? Math.min(100, (opcacheStats.memory_used_mb / (opcacheStats.memory_used_mb + opcacheStats.memory_free_mb)) * 100)
    : 0;

  $: valkeyHitRate = valkeyStats?.hit_rate ?? 0;
  $: opcacheHitRate = opcacheStats?.hit_rate ?? 0;

  // Valkey memory ring percentage (used vs peak)
  $: valkeyMemPct = valkeyStats
    ? Math.min(100, (valkeyStats.memory_used_mb / Math.max(valkeyStats.memory_peak_mb, valkeyStats.memory_used_mb, 1)) * 100)
    : 0;
</script>

<svelte:head><title>{$t('cache.title')} — JottiCP</title></svelte:head>

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->
{#if toastMsg}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg flex items-center gap-2 fade-up {toastType === 'success' ? 'text-green-400' : 'text-red-400'}">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/></svg>
    {/if}
    <span class="text-foreground">{toastMsg}</span>
  </div>
{/if}

<!-- ── Confirm flush-all modal ─────────────────────────────────────────────── -->
{#if confirmFlushAll}
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => confirmFlushAll = false} role="dialog" aria-modal="true" aria-labelledby="confirm-title">
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl fade-up">
      <div class="flex items-start gap-4 mb-6">
        <div class="p-2.5 bg-red-500/10 rounded-xl shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
          </svg>
        </div>
        <div>
          <h3 id="confirm-title" class="text-base font-semibold text-foreground mb-1 flex items-center gap-2"><svg class="w-4 h-4 text-primary shrink-0" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>Flush All Caches</h3>
          <p class="text-sm text-muted-foreground">This will flush OPcache and all Valkey keys for the selected site. Temporarily degraded performance while caches warm up.</p>
        </div>
      </div>
      <div class="flex justify-end gap-3">
        <button class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
                on:click={() => confirmFlushAll = false}>Cancel</button>
        <button class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
                on:click={flushAll} disabled={flushingAll}>
          {#if flushingAll}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/></svg>
            Flushing…
          {:else}
            Flush All
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ════════════════ OPcache File List Modal ════════════════ -->
{#if showOpcacheFiles}
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       role="dialog" aria-modal="true" aria-label="OPcache cached files">
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => showOpcacheFiles = false}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-3xl shadow-2xl max-h-[85vh] flex flex-col fade-up">
      <!-- Header -->
      <div class="flex items-center justify-between mb-4">
        <div>
          <h2 class="text-base font-semibold text-foreground">OPcache Cached Files</h2>
          <p class="text-xs text-muted-foreground mt-0.5">{opcacheFiles.length} file{opcacheFiles.length !== 1 ? 's' : ''} in cache</p>
        </div>
        <div class="flex items-center gap-2">
          <button
            on:click={invalidateAll}
            disabled={invalidatingFile === '__all__' || opcacheFiles.length === 0}
            class="h-8 px-3 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-xs font-medium hover:bg-red-500/20 inline-flex items-center gap-1.5 disabled:opacity-50 transition-all duration-150 active:scale-95"
          >
            {#if invalidatingFile === '__all__'}
              <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/></svg>
            {:else}
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
              </svg>
            {/if}
            Invalidate All
          </button>
          <button
            on:click={() => showOpcacheFiles = false}
            class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
            </svg>
          </button>
        </div>
      </div>

      <!-- Search -->
      <div class="relative mb-4">
        <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none"
             fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
        </svg>
        <input
          type="search"
          bind:value={opcacheFileSearch}
          placeholder="Filter file paths…"
          class="h-9 w-full pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
        />
      </div>

      <!-- File list -->
      <div class="flex-1 overflow-y-auto min-h-0">
        {#if opcacheFilesLoading}
          <div class="flex items-center justify-center py-16 text-muted-foreground text-sm gap-3">
            <svg class="w-5 h-5 animate-spin text-primary" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
            </svg>
            Loading cached files…
          </div>
        {:else if filteredOpcacheFiles.length === 0}
          <div class="flex flex-col items-center py-12 text-center">
            <div class="p-3 bg-muted/20 rounded-full mb-3">
              <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>
              </svg>
            </div>
            <p class="text-sm text-foreground font-medium mb-1">
              {opcacheFiles.length === 0 ? 'No cached files found' : 'No files match your search'}
            </p>
            <p class="text-xs text-muted-foreground">
              {opcacheFiles.length === 0 ? 'Cached files list requires API support — coming soon' : 'Try a different search term'}
            </p>
          </div>
        {:else}
          <table class="w-full">
            <thead class="bg-muted/50 sticky top-0">
              <tr>
                <th class="px-3 py-2.5 text-left text-xs text-muted-foreground uppercase">File path</th>
                <th class="px-3 py-2.5 text-right text-xs text-muted-foreground uppercase">Hits</th>
                <th class="px-3 py-2.5 text-right text-xs text-muted-foreground uppercase hidden sm:table-cell">Size</th>
                <th class="px-3 py-2.5 text-right text-xs text-muted-foreground uppercase">Action</th>
              </tr>
            </thead>
            <tbody>
              {#each filteredOpcacheFiles as file (file.path)}
                <tr class="border-t border-border hover:bg-muted/20 transition-colors">
                  <td class="px-3 py-2.5 text-xs">
                    <span
                      class="font-mono text-foreground/80 max-w-xs block truncate"
                      title={file.path}
                    >{file.path}</span>
                  </td>
                  <td class="px-3 py-2.5 text-xs text-right font-mono text-muted-foreground">{file.hits.toLocaleString()}</td>
                  <td class="px-3 py-2.5 text-xs text-right text-muted-foreground hidden sm:table-cell">{formatBytes(file.size_bytes)}</td>
                  <td class="px-3 py-2.5 text-right">
                    <button
                      on:click={() => invalidateFile(file.path)}
                      disabled={invalidatingFile === file.path}
                      class="h-7 px-2.5 rounded-md bg-red-500/10 text-red-400 border border-red-500/20 text-xs hover:bg-red-500/20 inline-flex items-center gap-1 disabled:opacity-50 transition-all duration-150 active:scale-95"
                      title="Invalidate this file"
                    >
                      {#if invalidatingFile === file.path}
                        <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/></svg>
                      {:else}
                        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
                        </svg>
                      {/if}
                      Invalidate
                    </button>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </div>
    </div>
  </div>
{/if}

<!-- ════════════════ Valkey Key Browser Modal ════════════════ -->
{#if showValkeyBrowser}
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       role="dialog" aria-modal="true" aria-label="Valkey key browser">
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => showValkeyBrowser = false}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-3xl shadow-2xl max-h-[85vh] flex flex-col fade-up">
      <!-- Header -->
      <div class="flex items-center justify-between mb-4">
        <div>
          <h2 class="text-base font-semibold text-foreground">Valkey Key Browser</h2>
          <p class="text-xs text-muted-foreground mt-0.5">{valkeyKeys.length} key{valkeyKeys.length !== 1 ? 's' : ''} found</p>
        </div>
        <button
          on:click={() => showValkeyBrowser = false}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <!-- Search -->
      <div class="flex gap-2 mb-4">
        <div class="relative flex-1">
          <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none"
               fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
          </svg>
          <input
            type="search"
            bind:value={valkeyKeySearch}
            placeholder="Pattern, e.g. session:* or user:*"
            on:keydown={(e) => { if (e.key === 'Enter') { valkeyKeysPage = 0; loadValkeyKeys(); } }}
            class="h-9 w-full pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
          />
        </div>
        <button
          on:click={() => { valkeyKeysPage = 0; loadValkeyKeys(); }}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          Search
        </button>
      </div>

      <!-- Keys list + value preview -->
      <div class="flex-1 flex gap-4 overflow-hidden min-h-0">
        <!-- Left: key list -->
        <div class="flex-1 overflow-y-auto min-h-0 border border-border rounded-xl">
          {#if valkeyKeysLoading}
            <div class="flex items-center justify-center py-16 text-muted-foreground text-sm gap-3">
              <svg class="w-5 h-5 animate-spin text-primary" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
              </svg>
              Loading keys…
            </div>
          {:else if valkeyKeys.length === 0}
            <div class="flex flex-col items-center py-12 text-center px-4">
              <div class="p-3 bg-muted/20 rounded-full mb-3">
                <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"/>
                </svg>
              </div>
              <p class="text-sm font-medium text-foreground mb-1">No keys found</p>
              <p class="text-xs text-muted-foreground">Key browser requires Redis/Valkey CLI access — coming soon</p>
            </div>
          {:else}
            <table class="w-full">
              <thead class="bg-muted/50 sticky top-0">
                <tr>
                  <th class="px-3 py-2.5 text-left text-xs text-muted-foreground uppercase">Key</th>
                  <th class="px-3 py-2.5 text-left text-xs text-muted-foreground uppercase hidden sm:table-cell">Type</th>
                  <th class="px-3 py-2.5 text-right text-xs text-muted-foreground uppercase">TTL</th>
                  <th class="px-3 py-2.5 text-right text-xs text-muted-foreground uppercase">Del</th>
                </tr>
              </thead>
              <tbody>
                {#each valkeyPagedKeys as k (k.key)}
                  <tr
                    class="border-t border-border hover:bg-muted/20 transition-colors cursor-pointer {valkeySelectedKey === k.key ? 'bg-primary/5' : ''}"
                    on:click={() => loadKeyValue(k.key)}
                    on:keydown={(e) => e.key === 'Enter' && loadKeyValue(k.key)}
                    tabindex="0"
                    role="row"
                  >
                    <td class="px-3 py-2.5">
                      <span class="font-mono text-xs text-foreground/80 max-w-[160px] block truncate" title={k.key}>{k.key}</span>
                    </td>
                    <td class="px-3 py-2.5 hidden sm:table-cell">
                      <span class="px-1.5 py-0.5 rounded text-xs font-medium border {keyTypeBadge(k.type)}">{k.type || 'str'}</span>
                    </td>
                    <td class="px-3 py-2.5 text-right text-xs text-muted-foreground font-mono">{ttlLabel(k.ttl ?? -1)}</td>
                    <td class="px-3 py-2.5 text-right">
                      <!-- svelte-ignore a11y-click-events-have-key-events -->
                      <button
                        on:click|stopPropagation={() => deleteValkeyKey(k.key)}
                        class="w-6 h-6 rounded flex items-center justify-center text-muted-foreground hover:bg-red-500/10 hover:text-red-400 transition-colors"
                        title="Delete key"
                      >
                        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
                        </svg>
                      </button>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>

            <!-- Pagination -->
            {#if valkeyTotalPages > 1}
              <div class="flex items-center justify-between px-3 py-2.5 border-t border-border text-xs text-muted-foreground">
                <span>Page {valkeyKeysPage + 1} of {valkeyTotalPages}</span>
                <div class="flex gap-1">
                  <button
                    on:click={() => valkeyKeysPage = Math.max(0, valkeyKeysPage - 1)}
                    disabled={valkeyKeysPage === 0}
                    class="h-7 px-2.5 rounded border border-border hover:bg-muted disabled:opacity-40 transition-all duration-150 active:scale-95"
                  >Prev</button>
                  <button
                    on:click={() => valkeyKeysPage = Math.min(valkeyTotalPages - 1, valkeyKeysPage + 1)}
                    disabled={valkeyKeysPage >= valkeyTotalPages - 1}
                    class="h-7 px-2.5 rounded border border-border hover:bg-muted disabled:opacity-40 transition-all duration-150 active:scale-95"
                  >Next</button>
                </div>
              </div>
            {/if}
          {/if}
        </div>

        <!-- Right: value preview -->
        {#if valkeySelectedKey}
          <div class="w-64 flex flex-col gap-2 shrink-0">
            <div class="text-xs font-medium text-muted-foreground truncate" title={valkeySelectedKey}>{valkeySelectedKey}</div>
            {#if valkeyKeyValueLoading}
              <div class="flex-1 rounded-xl bg-muted/20 flex items-center justify-center text-xs text-muted-foreground">
                Loading…
              </div>
            {:else}
              <pre class="flex-1 bg-[#0d1117] rounded-xl p-3 text-xs font-mono text-slate-300 overflow-auto leading-relaxed">{valkeyKeyValue || '(empty)'}</pre>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<div class="p-6 space-y-6 max-w-6xl">
  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400 fade-up">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadStats()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- ── Page header ─────────────────────────────────────────────────────── -->
  <div class="flex items-center justify-between flex-wrap gap-4 fade-up">
    <div class="flex items-center gap-3">
      <div class="p-2.5 bg-primary/10 rounded-xl">
        <svg class="w-5 h-5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z"/>
        </svg>
      </div>
      <div>
        <h1 class="text-xl font-bold text-foreground">{$t('cache.title')}</h1>
        <p class="text-sm text-muted-foreground">{$t('cache.subtitle')}</p>
      </div>
    </div>
    <button class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
            on:click={() => confirmFlushAll = true}>
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
      </svg>
      {$t('cache.flush_all')}
    </button>
  </div>

  {#if loading}
    <!-- Skeleton loader -->
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
      {#each [1,2] as _}
        <div class="bg-card border border-border rounded-xl p-6 space-y-4 animate-pulse">
          <div class="h-5 bg-muted rounded w-1/3"></div>
          <div class="h-3 bg-muted rounded w-full"></div>
          <div class="h-3 bg-muted rounded w-2/3"></div>
          <div class="h-3 bg-muted rounded w-1/2"></div>
        </div>
      {/each}
    </div>
  {:else}

  <!-- ── OPcache card ─────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center justify-between mb-5">
      <div class="flex items-center gap-3">
        <div class="p-2 bg-blue-500/10 rounded-lg">
          <svg class="w-4 h-4 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"/>
          </svg>
        </div>
        <h2 class="text-base font-semibold text-foreground">OPcache</h2>
      </div>
      <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border
        {opcacheStats?.enabled ? 'bg-green-500/10 text-green-400 border-green-500/20' : 'bg-muted/20 text-muted-foreground border-border'}">
        <span class="w-1.5 h-1.5 rounded-full {opcacheStats?.enabled ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
        {opcacheStats?.enabled ? 'Active' : 'Disabled'}
      </span>
    </div>

    {#if opcacheStats?.enabled}
      <div class="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-5">
        <!-- Hit rate donut -->
        <div class="col-span-2 sm:col-span-1 flex flex-col items-start gap-2">
          <span class="text-xs text-muted-foreground">Hit Rate</span>
          <div class="flex items-center gap-3">
            <!-- CSS-only donut chart -->
            <svg class="w-16 h-16 -rotate-90" viewBox="0 0 36 36">
              <circle cx="18" cy="18" r="15.9155" fill="none" stroke="currentColor"
                      stroke-width="2.5" class="text-muted/30"/>
              <circle cx="18" cy="18" r="15.9155" fill="none" stroke="currentColor"
                      stroke-width="2.5"
                      class="{opcacheHitRate > 90 ? 'text-green-400' : opcacheHitRate > 70 ? 'text-amber-400' : 'text-red-400'}"
                      stroke-dasharray="{opcacheHitRate} {100 - opcacheHitRate}"
                      stroke-dashoffset="0"
                      stroke-linecap="round"
                      style="transition: stroke-dasharray 0.8s ease-out"/>
              <text x="18" y="20.5" text-anchor="middle" font-size="6.5" font-weight="600"
                    fill="currentColor" class="rotate-90 origin-center text-foreground"
                    transform="rotate(90, 18, 18)">{Math.round(opcacheHitRate)}%</text>
            </svg>
            <div>
              <span class="text-2xl font-bold {hitRateColor(opcacheStats.hit_rate)}">{fmt(opcacheStats.hit_rate)}%</span>
              <span class="mt-1 block text-xs px-2 py-0.5 rounded-full font-medium border {hitRateBadge(opcacheStats.hit_rate)}">
                {opcacheStats.hit_rate >= 90 ? 'Excellent' : opcacheStats.hit_rate >= 70 ? 'Good' : 'Low'}
              </span>
            </div>
          </div>
        </div>
        <div class="flex flex-col">
          <span class="text-xs text-muted-foreground mb-1">Cached Files</span>
          <span class="text-xl font-semibold text-foreground">{opcacheStats.cached_scripts}</span>
        </div>
        <div class="flex flex-col">
          <span class="text-xs text-muted-foreground mb-1">JIT</span>
          <span class="text-xl font-semibold {opcacheStats.jit_enabled ? 'text-green-400' : 'text-muted-foreground'}">
            {opcacheStats.jit_enabled ? 'On' : 'Off'}
          </span>
          {#if opcacheStats.jit_enabled && opcacheStats.jit_buffer_size}
            <span class="text-xs text-muted-foreground">{(opcacheStats.jit_buffer_size / 1048576).toFixed(0)} MB buf</span>
          {/if}
        </div>
        <!-- Memory ring -->
        <div class="flex flex-col items-start gap-2">
          <span class="text-xs text-muted-foreground">Memory</span>
          <div class="flex items-center gap-2">
            <svg class="w-12 h-12 -rotate-90" viewBox="0 0 36 36">
              <circle cx="18" cy="18" r="15.9155" fill="none" stroke="currentColor"
                      stroke-width="2.5" class="text-muted/30"/>
              <circle cx="18" cy="18" r="15.9155" fill="none" stroke="currentColor"
                      stroke-width="2.5"
                      class="{opcacheUsedPct > 85 ? 'text-red-400' : opcacheUsedPct > 65 ? 'text-amber-400' : 'text-blue-400'}"
                      stroke-dasharray="{opcacheUsedPct} {100 - opcacheUsedPct}"
                      stroke-dashoffset="0"
                      stroke-linecap="round"
                      style="transition: stroke-dasharray 0.8s ease-out"/>
            </svg>
            <div>
              <span class="text-lg font-semibold text-foreground">{fmt(opcacheStats.memory_used_mb)} MB</span>
              <span class="text-xs text-muted-foreground block">of {fmt(opcacheStats.memory_used_mb + opcacheStats.memory_free_mb, 0)} MB</span>
            </div>
          </div>
        </div>
      </div>

      <!-- Memory bar -->
      <div class="mb-5">
        <div class="flex justify-between text-xs text-muted-foreground mb-1.5">
          <span>Memory Usage</span>
          <span>{fmt(opcacheStats.memory_used_mb)} MB used / {fmt(opcacheStats.memory_free_mb)} MB free</span>
        </div>
        <div class="h-2 rounded-full bg-muted/30 overflow-hidden">
          <div class="h-2 rounded-full transition-all duration-500
            {opcacheUsedPct > 85 ? 'bg-red-400' : opcacheUsedPct > 65 ? 'bg-amber-400' : 'bg-primary'}"
               style="width:{opcacheUsedPct}%"></div>
        </div>
      </div>

      <div class="flex flex-wrap gap-3 items-center">
        <button class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
                on:click={flushOpcache} disabled={flushing || !selectedSiteId}>
          {#if flushing}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/></svg>
          {:else}
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
          {/if}
          Flush OPcache
        </button>
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
          on:click={openOpcacheFiles}
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>
          </svg>
          View Cached Files
        </button>
      </div>
    {:else}
      <div class="flex flex-col items-center py-8 text-center">
        <div class="p-3 bg-muted/20 rounded-full mb-3">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"/>
          </svg>
        </div>
        <p class="text-sm text-muted-foreground">OPcache is not active on this server.</p>
      </div>
    {/if}
  </div>

  <!-- ── Valkey / Redis card ─────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center justify-between mb-5">
      <div class="flex items-center gap-3">
        <div class="p-2 bg-red-500/10 rounded-lg">
          <svg class="w-4 h-4 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/>
          </svg>
        </div>
        <div>
          <h2 class="text-base font-semibold text-foreground">Valkey</h2>
          {#if valkeyStats?.version && valkeyStats.version !== 'unknown'}
            <span class="text-xs text-muted-foreground">v{valkeyStats.version}</span>
          {/if}
        </div>
      </div>
      <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border
        {valkeyStats?.connected ? 'bg-green-500/10 text-green-400 border-green-500/20' : 'bg-red-500/10 text-red-400 border-red-500/20'}">
        <span class="w-1.5 h-1.5 rounded-full {valkeyStats?.connected ? 'bg-green-400 animate-pulse' : 'bg-red-400'}"></span>
        {valkeyStats?.connected ? 'Connected' : 'Disconnected'}
      </span>
    </div>

    {#if valkeyStats?.connected}
      <div class="grid grid-cols-2 sm:grid-cols-4 gap-4 mb-5">
        <!-- Hit rate donut -->
        <div class="flex flex-col items-start gap-2">
          <span class="text-xs text-muted-foreground">Hit Rate</span>
          <div class="flex items-center gap-3">
            <svg class="w-16 h-16 -rotate-90" viewBox="0 0 36 36">
              <circle cx="18" cy="18" r="15.9155" fill="none" stroke="currentColor"
                      stroke-width="2.5" class="text-muted/30"/>
              <circle cx="18" cy="18" r="15.9155" fill="none" stroke="currentColor"
                      stroke-width="2.5"
                      class="{valkeyHitRate > 90 ? 'text-green-400' : valkeyHitRate > 70 ? 'text-amber-400' : 'text-red-400'}"
                      stroke-dasharray="{valkeyHitRate} {100 - valkeyHitRate}"
                      stroke-dashoffset="0"
                      stroke-linecap="round"
                      style="transition: stroke-dasharray 0.8s ease-out"/>
              <text x="18" y="20.5" text-anchor="middle" font-size="6.5" font-weight="600"
                    fill="currentColor" transform="rotate(90, 18, 18)">{Math.round(valkeyHitRate)}%</text>
            </svg>
            <div>
              <span class="text-2xl font-bold {hitRateColor(valkeyStats.hit_rate)}">{fmt(valkeyStats.hit_rate)}%</span>
              <span class="mt-1 block text-xs px-2 py-0.5 rounded-full font-medium border {hitRateBadge(valkeyStats.hit_rate)}">
                {valkeyHitRate >= 90 ? 'Excellent' : valkeyHitRate >= 70 ? 'Good' : 'Low'}
              </span>
            </div>
          </div>
        </div>
        <div class="flex flex-col">
          <span class="text-xs text-muted-foreground mb-1">Total Keys</span>
          <span class="text-xl font-semibold text-foreground">{valkeyStats.keys}</span>
        </div>
        <!-- Memory ring -->
        <div class="flex flex-col items-start gap-1">
          <span class="text-xs text-muted-foreground">Memory</span>
          <div class="flex items-center gap-2">
            <svg class="w-12 h-12 -rotate-90" viewBox="0 0 36 36">
              <circle cx="18" cy="18" r="15.9155" fill="none" stroke="currentColor"
                      stroke-width="2.5" class="text-muted/30"/>
              <circle cx="18" cy="18" r="15.9155" fill="none" stroke="currentColor"
                      stroke-width="2.5"
                      class="{valkeyMemPct > 85 ? 'text-red-400' : valkeyMemPct > 65 ? 'text-amber-400' : 'text-red-400'}"
                      stroke-dasharray="{valkeyMemPct} {100 - valkeyMemPct}"
                      stroke-dashoffset="0"
                      stroke-linecap="round"
                      style="transition: stroke-dasharray 0.8s ease-out"/>
            </svg>
            <div>
              <span class="text-lg font-semibold text-foreground">{fmt(valkeyStats.memory_used_mb)} MB</span>
              <span class="text-xs text-muted-foreground block">peak {fmt(valkeyStats.memory_peak_mb)} MB</span>
            </div>
          </div>
        </div>
        <div class="flex flex-col">
          <span class="text-xs text-muted-foreground mb-1">Uptime</span>
          <span class="text-xl font-semibold text-foreground">{uptimeLabel(valkeyStats.uptime_seconds)}</span>
          <span class="text-xs text-muted-foreground">{valkeyStats.connected_clients} client{valkeyStats.connected_clients === 1 ? '' : 's'}</span>
        </div>
      </div>

      <!-- Memory bar -->
      <div class="mb-5">
        <div class="h-2 rounded-full bg-muted/30 overflow-hidden">
          <div class="h-2 rounded-full bg-red-400 transition-all duration-500" style="width:{valkeyMemPct}%"></div>
        </div>
      </div>

      <!-- Flush actions + browse keys -->
      <div class="flex flex-wrap gap-3 items-end">
        <button class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
                on:click={flushValkey} disabled={flushing || !selectedSiteId}>
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
          </svg>
          Flush All Keys
        </button>
        <!-- Browse Keys button -->
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
          on:click={openValkeyBrowser}
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"/>
          </svg>
          Browse Keys
        </button>
        <div class="flex items-center gap-2 flex-1 min-w-48">
          <input
            class="h-9 flex-1 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
            type="text"
            placeholder="Pattern, e.g. session:*"
            bind:value={valkeyPattern}
            on:keydown={(e) => e.key === 'Enter' && flushByPattern()}
          />
          <button class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
                  on:click={flushByPattern} disabled={flushing}>
            Flush Pattern
          </button>
        </div>
      </div>
    {:else}
      <div class="flex flex-col items-center py-8 text-center">
        <div class="p-3 bg-muted/20 rounded-full mb-3">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/>
          </svg>
        </div>
        <p class="text-sm font-medium text-foreground mb-1">Cannot connect to Valkey</p>
        <p class="text-xs text-muted-foreground">Check that the Valkey service is running on this server.</p>
      </div>
    {/if}
  </div>

  <!-- ── Browser Cache section ───────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center gap-3 mb-5">
      <div class="p-2 bg-amber-500/10 rounded-lg">
        <svg class="w-4 h-4 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9"/>
        </svg>
      </div>
      <div>
        <h2 class="text-base font-semibold text-foreground">Browser Cache Policy</h2>
        <p class="text-sm text-muted-foreground">Controls HTTP cache headers sent to visitors</p>
      </div>
    </div>

    <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3 mb-5">
      {#each [
        { value: 'aggressive', icon: '🚀', label: 'Aggressive', sublabel: '1 year', desc: 'Static assets cached forever — requires cache-busting on deploys' },
        { value: 'balanced',   icon: '⚖️', label: 'Balanced',   sublabel: '1 week', desc: 'Good for most production sites', recommended: true },
        { value: 'minimal',    icon: '🔄', label: 'Minimal',    sublabel: '1 hour', desc: 'Frequent content updates, short-lived cache' },
        { value: 'disabled',   icon: '❌', label: 'Disabled',   sublabel: 'No cache', desc: 'Development mode — no browser caching' },
      ] as preset}
        <label class="relative flex flex-col gap-1.5 p-4 rounded-xl border-2 cursor-pointer transition-all duration-200
          {cachePreset === preset.value
            ? 'border-primary bg-primary/5'
            : 'border-border hover:border-primary/50 hover:bg-muted/30'}">
          <input type="radio" class="sr-only" name="cachePreset" value={preset.value} bind:group={cachePreset} />
          {#if preset.recommended}
            <span class="absolute top-2 right-2 text-xs px-1.5 py-0.5 rounded-full bg-primary/10 text-primary font-medium">Recommended</span>
          {/if}
          <span class="text-lg">{preset.icon}</span>
          <div>
            <span class="block text-sm font-semibold text-foreground">{preset.label}</span>
            <span class="block text-xs font-medium text-primary">{preset.sublabel}</span>
          </div>
          <span class="text-xs text-muted-foreground leading-relaxed">{preset.desc}</span>
          {#if cachePreset === preset.value}
            <div class="absolute top-2 left-2 w-2 h-2 rounded-full bg-primary"></div>
          {/if}
        </label>
      {/each}
    </div>

    <button class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
            on:click={applyPreset} disabled={!selectedSiteId}>
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
      </svg>
      Apply Preset
    </button>

    <!-- Per-site overrides expandable section -->
    {#if sites.length > 0}
      <div class="mt-6 border-t border-border pt-5">
        <button
          on:click={() => siteOverridesExpanded = !siteOverridesExpanded}
          class="flex items-center gap-2 text-sm font-medium text-foreground hover:text-primary transition-colors w-full text-left"
        >
          <svg class="w-4 h-4 transition-transform duration-200 {siteOverridesExpanded ? 'rotate-180' : ''}"
               fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
          </svg>
          Per-site overrides
          <span class="ml-1 text-xs text-muted-foreground font-normal">
            ({Object.values(siteOverrides).filter(Boolean).length} custom)
          </span>
        </button>

        {#if siteOverridesExpanded}
          <div class="mt-4 rounded-xl border border-border overflow-hidden fade-up">
            <table class="w-full">
              <thead class="bg-muted/50">
                <tr>
                  <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase">Site domain</th>
                  <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden sm:table-cell">Global preset</th>
                  <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase">Override</th>
                  <th class="px-4 py-3 text-right text-xs text-muted-foreground uppercase">Action</th>
                </tr>
              </thead>
              <tbody>
                {#each sites as site}
                  {@const override = siteOverrides[site.id] || ''}
                  <tr class="border-t border-border hover:bg-muted/20 transition-colors">
                    <td class="px-4 py-3 text-sm font-mono text-foreground">{site.domain}</td>
                    <td class="px-4 py-3 hidden sm:table-cell">
                      <span class="text-xs text-muted-foreground capitalize">{cachePreset}</span>
                    </td>
                    <td class="px-4 py-3">
                      <select
                        bind:value={siteOverrides[site.id]}
                        class="h-8 rounded-lg border border-border bg-background px-2 text-xs text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                      >
                        <option value="">Use global ({cachePreset})</option>
                        <option value="aggressive">Aggressive (1 year)</option>
                        <option value="balanced">Balanced (1 week)</option>
                        <option value="minimal">Minimal (1 hour)</option>
                        <option value="disabled">Disabled</option>
                      </select>
                    </td>
                    <td class="px-4 py-3 text-right">
                      <div class="flex items-center gap-1.5 justify-end">
                        <button
                          on:click={() => applyPerSitePreset(site.id)}
                          disabled={!siteOverrides[site.id]}
                          class="h-7 px-2.5 rounded-md bg-primary/10 text-primary text-xs font-medium hover:bg-primary/20 inline-flex items-center gap-1 disabled:opacity-40 disabled:cursor-not-allowed transition-all duration-150 active:scale-95"
                        >Apply</button>
                        {#if siteOverrides[site.id]}
                          <button
                            on:click={() => resetSiteOverride(site.id)}
                            class="h-7 px-2.5 rounded-md border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1 transition-all duration-150 active:scale-95"
                          >Reset</button>
                        {/if}
                      </div>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <!-- ── Site-specific flush ─────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-6 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">
    <div class="flex items-center gap-3 mb-5">
      <div class="p-2 bg-purple-500/10 rounded-lg">
        <svg class="w-4 h-4 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3.055 11H5a2 2 0 012 2v1a2 2 0 002 2 2 2 0 012 2v2.945M8 3.935V5.5A2.5 2.5 0 0010.5 8h.5a2 2 0 012 2 2 2 0 104 0 2 2 0 012-2h1.064M15 20.488V18a2 2 0 012-2h3.064"/>
        </svg>
      </div>
      <div>
        <h2 class="text-base font-semibold text-foreground">Site-Specific Cache Clear</h2>
        <p class="text-sm text-muted-foreground">Flush all cache layers for a single site</p>
      </div>
    </div>
    <div class="flex flex-wrap gap-3 items-end">
      <div class="flex-1 min-w-48">
        <label class="block text-xs text-muted-foreground mb-1.5">Site</label>
        <select class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                bind:value={selectedSiteId}>
          {#if sites.length === 0}
            <option value="">No sites found</option>
          {/if}
          {#each sites as s}
            <option value={s.id}>{s.domain}</option>
          {/each}
        </select>
      </div>
      <button class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
              on:click={() => confirmFlushAll = true} disabled={!selectedSiteId || flushingAll}>
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
        </svg>
        Flush Site Cache
      </button>
    </div>
  </div>

  {/if}<!-- end of !loading -->
</div>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px) }
    to   { opacity: 1; transform: none }
  }
  .fade-up {
    animation: fadeUp 0.25s ease-out both;
  }
</style>
