<script lang="ts">
  import { onMount, createEventDispatcher } from 'svelte';
  import { get } from 'svelte/store';
  import { api } from '$api/client';
  import type { Site, SiteStats, AppInstall } from '$api/client';
  import { auth } from '$lib/stores/auth';
  import StatusBadge from '$lib/components/ui/StatusBadge.svelte';

  // ── Props ────────────────────────────────────────────────────────────────────

  export let siteId: string;
  export let site: Site;

  // ── Event dispatcher ─────────────────────────────────────────────────────────

  const dispatch = createEventDispatcher<{ siteUpdated: void }>();

  // ── State ────────────────────────────────────────────────────────────────────

  let stats: SiteStats | null = null;
  let statsLoading = true;
  let statsError = false;

  let apps: Array<{ id: string; site_id: string; app_id: string; version: string; status: string; installed_at: string | null; admin_url: string | null }> = [];
  let appsLoading = true;

  let flushLoading = false;
  let sslLoading = false;
  let suspendLoading = false;
  let confirmSuspend = false;
  let updatingAppId: string | null = null;

  // Tags
  let tags: string[] = [];
  let tagsLoading = true;
  let newTag = '';
  let savingTags = false;

  // Clone modal
  let showCloneModal = false;
  let cloneTargetDomain = '';
  let cloneCopyDb = true;
  let cloneLoading = false;
  let cloneError = '';
  let cloneResult: string | null = null;

  interface Toast { message: string; type: 'success' | 'error' }
  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Auth helpers ──────────────────────────────────────────────────────────────

  function authHeaders(): Record<string, string> {
    const token = get(auth).token;
    return token ? { Authorization: 'Bearer ' + token } : {};
  }

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
    return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB';
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' });
  }

  function sslDaysLeft(expiresAt: string): number {
    return Math.ceil((new Date(expiresAt).getTime() - Date.now()) / 86400000);
  }

  function cpuColor(pct: number): string {
    if (pct >= 90) return 'bg-red-500';
    if (pct >= 70) return 'bg-yellow-500';
    return 'bg-blue-500';
  }

  function showToast(message: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 4000);
  }

  // ── Data loading ──────────────────────────────────────────────────────────────

  onMount(async () => {
    await Promise.all([loadStats(), loadApps(), loadTags()]);
  });

  async function loadStats() {
    statsLoading = true;
    statsError = false;
    try {
      stats = await api.sites.stats(siteId);
    } catch (err) {
      console.error('failed to load site stats:', err);
      statsError = true;
    } finally {
      statsLoading = false;
    }
  }

  async function loadApps() {
    appsLoading = true;
    try {
      apps = await api.apps.listInstalled(siteId);
    } catch {
      apps = [];
    } finally {
      appsLoading = false;
    }
  }

  async function loadTags() {
    tagsLoading = true;
    try {
      const res = await api.siteExtras.getTags(siteId);
      tags = res.tags ?? [];
    } catch {
      tags = [];
    } finally {
      tagsLoading = false;
    }
  }

  function addTag() {
    const t = newTag.trim().toLowerCase().replace(/\s+/g, '-');
    if (!t || tags.includes(t)) { newTag = ''; return; }
    tags = [...tags, t];
    newTag = '';
    void saveTags();
  }

  function removeTag(tag: string) {
    tags = tags.filter(t => t !== tag);
    void saveTags();
  }

  async function saveTags() {
    savingTags = true;
    try {
      await api.siteExtras.setTags(siteId, tags);
    } catch {
      showToast('Failed to save tags', 'error');
    } finally {
      savingTags = false;
    }
  }

  // ── Actions ───────────────────────────────────────────────────────────────────

  async function flushCache() {
    flushLoading = true;
    try {
      const [opcRes, valkeyRes] = await Promise.all([
        fetch(`/api/v1/cache/opcache/${siteId}/flush`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', ...authHeaders() },
          body: '{}',
        }),
        fetch(`/api/v1/cache/valkey/${siteId}/flush`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json', ...authHeaders() },
          body: '{}',
        }),
      ]);
      if (!opcRes.ok || !valkeyRes.ok) throw new Error('Cache flush returned an error');
      showToast('Cache flushed successfully', 'success');
    } catch {
      showToast('Failed to flush cache', 'error');
    } finally {
      flushLoading = false;
    }
  }

  async function renewSSL() {
    sslLoading = true;
    try {
      await api.sites.issueSSL(siteId);
      showToast('SSL renewal started — check back in a moment', 'success');
    } catch {
      showToast('Failed to start SSL renewal', 'error');
    } finally {
      sslLoading = false;
    }
  }

  async function toggleSuspend() {
    suspendLoading = true;
    try {
      if (site.status === 'active') {
        await api.sites.suspend(siteId);
        showToast('Site suspended', 'success');
      } else {
        await api.sites.unsuspend(siteId);
        showToast('Site unsuspended', 'success');
      }
      dispatch('siteUpdated');
    } catch {
      showToast(site.status === 'active' ? 'Failed to suspend site' : 'Failed to unsuspend site', 'error');
    } finally {
      suspendLoading = false;
      confirmSuspend = false;
    }
  }

  async function cloneSite() {
    cloneError = '';
    const d = cloneTargetDomain.trim().toLowerCase().replace(/^https?:\/\//, '').replace(/\/$/, '');
    if (!d) { cloneError = 'Target domain is required'; return; }
    if (!/^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$/.test(d)) {
      cloneError = 'Invalid domain format (e.g. clone.example.com)'; return;
    }
    cloneLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/clone`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...authHeaders() },
        body: JSON.stringify({ target_domain: d, copy_db: cloneCopyDb }),
      });
      const data = await res.json() as { message?: string };
      if (!res.ok) throw new Error((data as { message?: string }).message ?? `HTTP ${res.status}`);
      cloneResult = `Clone of ${site.domain} → ${d} is being created. Check back in a moment.`;
      showToast('Clone started successfully', 'success');
    } catch (err: unknown) {
      cloneError = (err as { message?: string }).message ?? 'Clone failed';
    } finally {
      cloneLoading = false;
    }
  }

  async function updateApp(appId: string) {
    updatingAppId = appId;
    try {
      await api.apps.update(appId);
      showToast('App update started', 'success');
      await loadApps();
    } catch {
      showToast('Failed to update app', 'error');
    } finally {
      updatingAppId = null;
    }
  }
</script>

<!-- ── Stats row ──────────────────────────────────────────────────────────────── -->
<div class="tab-content space-y-6">

  <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
    {#if statsLoading}
      {#each [0, 1, 2, 3] as _}
        <div class="bg-card border border-border rounded-xl p-4 space-y-3">
          <div class="animate-pulse bg-muted rounded h-3 w-16"></div>
          <div class="animate-pulse bg-muted rounded h-6 w-24"></div>
          <div class="animate-pulse bg-muted rounded h-2 w-full"></div>
        </div>
      {/each}
    {:else if statsError}
      <div class="col-span-full bg-card border border-amber-500/30 rounded-xl px-5 py-4 flex items-center justify-between gap-4">
        <div class="flex items-center gap-3 text-sm">
          <svg class="w-4 h-4 text-amber-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/></svg>
          <span class="text-muted-foreground">Stats unavailable — orbit-agent may be offline or this site has no metrics yet</span>
        </div>
        <button on:click={loadStats} class="h-8 px-3 rounded-lg border border-border text-xs font-medium hover:bg-muted transition-colors">Retry</button>
      </div>
    {:else if stats}
      <!-- CPU -->
      <div class="relative bg-card border border-border rounded-xl p-4 card-hover overflow-hidden">
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/40 to-transparent"></div>
        <p class="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-2">CPU Usage</p>
        <p class="text-xl font-bold text-foreground mb-3">{stats.cpu_usage_pct.toFixed(1)}%</p>
        <div class="h-2 bg-muted rounded-full overflow-hidden">
          <div
            class="h-full rounded-full transition-all {cpuColor(stats.cpu_usage_pct)}"
            style="width: {Math.min(stats.cpu_usage_pct, 100)}%"
          ></div>
        </div>
      </div>

      <!-- RAM -->
      <div class="relative bg-card border border-border rounded-xl p-4 card-hover overflow-hidden">
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/40 to-transparent"></div>
        <p class="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-2">Memory</p>
        <p class="text-xl font-bold text-foreground">{formatBytes(stats.memory_bytes)}</p>
        <p class="text-xs text-muted-foreground mt-1">RSS resident</p>
      </div>

      <!-- Disk -->
      <div class="relative bg-card border border-border rounded-xl p-4 card-hover overflow-hidden">
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/40 to-transparent"></div>
        <p class="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-2">Disk Used</p>
        <p class="text-xl font-bold text-foreground">{formatBytes(stats.disk_bytes)}</p>
        <p class="text-xs text-muted-foreground mt-1">Site files + DB</p>
      </div>

      <!-- Requests -->
      <div class="relative bg-card border border-border rounded-xl p-4 card-hover overflow-hidden">
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/40 to-transparent"></div>
        <p class="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-2">Requests / hr</p>
        <p class="text-xl font-bold text-foreground">{stats.request_count_1h.toLocaleString()}</p>
        <p class="text-xs text-muted-foreground mt-1">Last 60 minutes</p>
      </div>
    {:else}
      <!-- Stats unavailable state -->
      {#each ['CPU Usage', 'Memory', 'Disk Used', 'Requests / hr'] as label}
        <div class="bg-card border border-border rounded-xl p-4">
          <p class="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-2">{label}</p>
          <p class="text-sm text-muted-foreground">Unavailable</p>
        </div>
      {/each}
    {/if}
  </div>

  <!-- ── Site info card ──────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden">
    <div class="px-4 py-3 border-b border-border">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 21a9.004 9.004 0 008.716-6.747M12 21a9.004 9.004 0 01-8.716-6.747M12 21c2.485 0 4.5-4.03 4.5-9S14.485 3 12 3m0 18c-2.485 0-4.5-4.03-4.5-9S9.515 3 12 3m0 0a8.997 8.997 0 017.843 4.582M12 3a8.997 8.997 0 00-7.843 4.582m15.686 0A11.953 11.953 0 0112 10.5c-2.998 0-5.74-1.1-7.843-2.918m15.686 0A8.959 8.959 0 0121 12c0 .778-.099 1.533-.284 2.253M3.284 14.253A8.959 8.959 0 013 12c0-.778.099-1.533.284-2.253"/>
          </svg>
        </div>
        <h3 class="text-sm font-semibold text-foreground">Site Information</h3>
      </div>
    </div>
    <dl class="grid grid-cols-1 lg:grid-cols-2 divide-y lg:divide-y-0 lg:divide-x divide-border">
      <!-- Left column -->
      <div class="divide-y divide-border">
        <!-- Domain -->
        <div class="flex items-start gap-3 px-4 py-3">
          <dt class="w-28 shrink-0 text-xs font-medium text-muted-foreground pt-0.5 flex items-center gap-1.5">
            <svg class="w-3 h-3 text-muted-foreground/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M13.19 8.688a4.5 4.5 0 011.242 7.244l-4.5 4.5a4.5 4.5 0 01-6.364-6.364l1.757-1.757m13.35-.622l1.757-1.757a4.5 4.5 0 00-6.364-6.364l-4.5 4.5a4.5 4.5 0 001.242 7.244"/>
            </svg>
            Domain
          </dt>
          <dd class="flex items-center gap-1.5 text-sm text-foreground font-medium">
            {site.domain}
            <a
              href="https://{site.domain}"
              target="_blank"
              rel="noopener noreferrer"
              class="text-muted-foreground hover:text-primary transition-colors"
              aria-label="Open {site.domain} in new tab"
            >
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
              </svg>
            </a>
          </dd>
        </div>

        <!-- Status -->
        <div class="flex items-center gap-3 px-4 py-3">
          <dt class="w-28 shrink-0 text-xs font-medium text-muted-foreground flex items-center gap-1.5">
            <svg class="w-3 h-3 text-muted-foreground/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M9 12.75L11.25 15 15 9.75M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
            </svg>
            Status
          </dt>
          <dd>
            <StatusBadge status={site.status} />
          </dd>
        </div>

        <!-- PHP version -->
        <div class="flex items-center gap-3 px-4 py-3">
          <dt class="w-28 shrink-0 text-xs font-medium text-muted-foreground flex items-center gap-1.5">
            <span class="text-[10px] font-bold text-muted-foreground/70 leading-none">PHP</span>
            PHP Version
          </dt>
          <dd class="text-sm text-foreground">PHP {site.php_version}</dd>
        </div>

        <!-- Web server -->
        <div class="flex items-center gap-3 px-4 py-3">
          <dt class="w-28 shrink-0 text-xs font-medium text-muted-foreground flex items-center gap-1.5">
            <svg class="w-3 h-3 text-muted-foreground/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/>
            </svg>
            Web Server
          </dt>
          <dd class="text-sm text-foreground">{site.web_server.toUpperCase()}</dd>
        </div>
      </div>

      <!-- Right column -->
      <div class="divide-y divide-border">
        <!-- SSL -->
        <div class="flex items-start gap-3 px-4 py-3">
          <dt class="w-28 shrink-0 text-xs font-medium text-muted-foreground pt-0.5 flex items-center gap-1.5">
            <svg class="w-3 h-3 text-muted-foreground/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z"/>
            </svg>
            SSL
          </dt>
          <dd class="text-sm">
            {#if site.ssl_status === 'active'}
              <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">
                Active
              </span>
              {#if site.ssl_expires_at}
                <span class="text-xs text-muted-foreground ml-2">
                  expires in {sslDaysLeft(site.ssl_expires_at)} days
                </span>
              {/if}
            {:else if site.ssl_status === 'expired'}
              <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-red-500/10 text-red-400 border border-red-500/20">
                Expired
              </span>
            {:else if site.ssl_status === 'pending'}
              <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-yellow-500/10 text-yellow-400 border border-yellow-500/20">
                Pending
              </span>
            {:else}
              <span class="text-muted-foreground">None</span>
            {/if}
          </dd>
        </div>

        <!-- Server -->
        <div class="flex items-center gap-3 px-4 py-3">
          <dt class="w-28 shrink-0 text-xs font-medium text-muted-foreground flex items-center gap-1.5">
            <svg class="w-3 h-3 text-muted-foreground/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01"/>
            </svg>
            Server
          </dt>
          <dd class="text-sm text-foreground">{site.server_label ?? 'Local'}</dd>
        </div>

        <!-- Created -->
        <div class="flex items-center gap-3 px-4 py-3">
          <dt class="w-28 shrink-0 text-xs font-medium text-muted-foreground flex items-center gap-1.5">
            <svg class="w-3 h-3 text-muted-foreground/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 6v6h4.5m4.5 0a9 9 0 11-18 0 9 9 0 0118 0z"/>
            </svg>
            Created
          </dt>
          <dd class="text-sm text-foreground">{formatDate(site.created_at)}</dd>
        </div>

        <!-- Unix user -->
        <div class="flex items-center gap-3 px-4 py-3">
          <dt class="w-28 shrink-0 text-xs font-medium text-muted-foreground flex items-center gap-1.5">
            <svg class="w-3 h-3 text-muted-foreground/60" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z"/>
            </svg>
            Unix User
          </dt>
          <dd class="text-sm text-foreground font-mono">{site.unix_user}</dd>
        </div>
      </div>
    </dl>
  </div>

  <!-- ── Quick actions ──────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-4">
    <div class="flex items-center gap-2.5 mb-4">
      <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
          <path stroke-linecap="round" stroke-linejoin="round" d="M3.75 13.5l10.5-11.25L12 10.5h8.25L9.75 21.75 12 13.5H3.75z"/>
        </svg>
      </div>
      <h3 class="text-sm font-semibold text-foreground">Quick Actions</h3>
    </div>
    <div class="flex flex-wrap items-center gap-2">

      <!-- Flush Cache -->
      <button
        on:click={flushCache}
        disabled={flushLoading}
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
               hover:bg-primary/90 transition-all duration-150 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed
               inline-flex items-center gap-2"
      >
        {#if flushLoading}
          <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
          </svg>
          Flushing...
        {:else}
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
          Flush Cache
        {/if}
      </button>

      <!-- Renew SSL -->
      <button
        on:click={renewSSL}
        disabled={sslLoading}
        class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
               hover:bg-muted transition-all duration-150 active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed
               inline-flex items-center gap-2"
      >
        {#if sslLoading}
          <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
          </svg>
          Renewing...
        {:else}
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
          </svg>
          Renew SSL
        {/if}
      </button>

      <!-- Suspend / Unsuspend -->
      {#if site.status === 'active'}
        {#if confirmSuspend}
          <!-- Inline warning — prevents accidental suspension -->
          <div class="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-destructive/10 border border-destructive/30">
            <svg class="w-3.5 h-3.5 text-destructive shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
            </svg>
            <span class="text-xs text-destructive font-medium">Site goes offline for all visitors.</span>
            <button
              on:click={toggleSuspend}
              disabled={suspendLoading}
              class="h-7 px-3 rounded-md bg-destructive text-destructive-foreground text-xs font-medium
                     hover:bg-destructive/90 disabled:opacity-50 inline-flex items-center gap-1.5"
            >
              {#if suspendLoading}
                <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                </svg>
              {/if}
              Confirm Suspend
            </button>
            <button
              on:click={() => confirmSuspend = false}
              class="h-7 px-2.5 rounded-md border border-border text-xs text-muted-foreground hover:bg-muted"
            >
              Cancel
            </button>
          </div>
        {:else}
          <button
            on:click={() => confirmSuspend = true}
            class="h-9 px-4 rounded-lg border border-destructive/40 text-destructive text-sm font-medium
                   bg-destructive/5 hover:bg-destructive/10 transition-all duration-150 active:scale-95
                   inline-flex items-center gap-2"
          >
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M10 9v6m4-6v6m7-3a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            Suspend
          </button>
        {/if}
      {:else if site.status === 'suspended'}
        <button
          on:click={toggleSuspend}
          disabled={suspendLoading}
          class="h-9 px-4 rounded-lg text-sm font-medium transition-all duration-150 active:scale-95
                 bg-green-600 text-white hover:bg-green-700
                 disabled:opacity-50 disabled:cursor-not-allowed
                 inline-flex items-center gap-2"
        >
          {#if suspendLoading}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
            </svg>
            Activating...
          {:else}
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M14.752 11.168l-3.197-2.132A1 1 0 0010 9.87v4.263a1 1 0 001.555.832l3.197-2.132a1 1 0 000-1.664z" />
              <path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
            Unsuspend
          {/if}
        </button>
      {/if}

      <!-- Visit Site -->
      <a
        href="https://{site.domain}"
        target="_blank"
        rel="noopener noreferrer"
        class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
               hover:bg-muted transition-all duration-150 active:scale-95 inline-flex items-center gap-2"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
        </svg>
        Visit Site
      </a>

      <!-- Clone Site -->
      <button
        on:click={() => { showCloneModal = true; cloneTargetDomain = ''; cloneError = ''; cloneResult = null; }}
        class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
               hover:bg-muted transition-all duration-150 active:scale-95 inline-flex items-center gap-2"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" />
        </svg>
        Clone Site
      </button>

    </div>
  </div>

  <!-- ── Tags ─────────────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-4">
    <div class="flex items-center justify-between gap-2.5 mb-3">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9.568 3H5.25A2.25 2.25 0 003 5.25v4.318c0 .597.237 1.17.659 1.591l9.581 9.581c.699.699 1.78.872 2.607.33a18.095 18.095 0 005.223-5.223c.542-.827.369-1.908-.33-2.607L9.568 3z"/>
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 6h.008v.008H6V6z"/>
          </svg>
        </div>
        <h3 class="text-sm font-semibold text-foreground">Tags</h3>
      </div>
      {#if savingTags}
        <svg class="w-3.5 h-3.5 animate-spin text-muted-foreground" fill="none" viewBox="0 0 24 24">
          <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
          <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
        </svg>
      {/if}
    </div>

    {#if tagsLoading}
      <div class="flex gap-2">
        {#each [0, 1, 2] as _}
          <div class="animate-pulse bg-muted rounded-full h-6 w-16"></div>
        {/each}
      </div>
    {:else}
      <div class="flex flex-wrap items-center gap-2">
        {#each tags as tag (tag)}
          <span class="inline-flex items-center gap-1 pl-3 pr-2 py-1 rounded-full text-xs font-medium bg-primary/10 text-primary border border-primary/20">
            {tag}
            <button
              type="button"
              on:click={() => removeTag(tag)}
              class="text-primary/60 hover:text-primary transition-colors ml-0.5"
              aria-label="Remove tag {tag}"
            >
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
              </svg>
            </button>
          </span>
        {/each}

        <!-- Add tag input -->
        <form
          on:submit|preventDefault={addTag}
          class="inline-flex items-center gap-1"
        >
          <input
            type="text"
            bind:value={newTag}
            placeholder="add tag…"
            maxlength="32"
            class="h-7 w-28 px-2 text-xs rounded-full border border-dashed border-border bg-transparent text-foreground
                   placeholder:text-muted-foreground/50 focus:outline-none focus:border-primary focus:ring-1 focus:ring-ring transition-colors"
          />
          {#if newTag.trim()}
            <button
              type="submit"
              class="h-7 px-2 text-xs rounded-full bg-primary/10 text-primary border border-primary/20 hover:bg-primary/20 transition-colors"
            >
              Add
            </button>
          {/if}
        </form>

        {#if tags.length === 0}
          <span class="text-xs text-muted-foreground">No tags — type above to add one</span>
        {/if}
      </div>
    {/if}
  </div>

  <!-- ── Site Health mini card ─────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-4">
    <div class="flex items-center gap-2.5 mb-3">
      <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
          <path stroke-linecap="round" stroke-linejoin="round" d="M9 12.75L11.25 15 15 9.75m-3-7.036A11.959 11.959 0 013.598 6 11.99 11.99 0 003 9.749c0 5.592 3.824 10.29 9 11.623 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.571-.598-3.751h-.152c-3.196 0-6.1-1.248-8.25-3.285z"/>
        </svg>
      </div>
      <h3 class="text-sm font-semibold text-foreground">Site Health</h3>
    </div>
    <div class="flex flex-wrap items-center gap-4">
      <!-- SSL status -->
      <div class="flex items-center gap-2">
        <span class="w-2 h-2 rounded-full {site.ssl_status === 'active' ? 'bg-green-400' : site.ssl_status === 'pending' ? 'bg-yellow-400 animate-pulse' : 'bg-red-400'}"></span>
        <span class="text-xs text-muted-foreground">SSL: <span class="text-foreground font-medium capitalize">{site.ssl_status ?? 'None'}</span></span>
      </div>
      <!-- OPcache -->
      <div class="flex items-center gap-2">
        <span class="w-2 h-2 rounded-full {stats ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
        <span class="text-xs text-muted-foreground">OPcache: <span class="text-foreground font-medium">{stats ? 'Active' : 'Unknown'}</span></span>
      </div>
      <!-- Last backup placeholder -->
      <div class="flex items-center gap-2">
        <span class="w-2 h-2 rounded-full bg-blue-400"></span>
        <span class="text-xs text-muted-foreground">Backups: <span class="text-foreground font-medium">Enabled</span></span>
      </div>
      <!-- Site status -->
      <div class="flex items-center gap-2">
        <span class="w-2 h-2 rounded-full {site.status === 'active' ? 'bg-green-400' : 'bg-red-400'}"></span>
        <span class="text-xs text-muted-foreground">Status: <span class="text-foreground font-medium capitalize">{site.status}</span></span>
      </div>
    </div>
  </div>

  <!-- ── Installed apps ─────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden">
    <div class="px-4 py-3 border-b border-border">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4"/>
          </svg>
        </div>
        <h3 class="text-sm font-semibold text-foreground">Installed Apps</h3>
      </div>
    </div>

    {#if appsLoading}
      <div class="divide-y divide-border">
        {#each [0, 1] as _}
          <div class="flex items-center gap-4 px-4 py-4">
            <div class="animate-pulse bg-muted rounded-lg w-10 h-10 shrink-0"></div>
            <div class="flex-1 space-y-2">
              <div class="animate-pulse bg-muted rounded h-4 w-32"></div>
              <div class="animate-pulse bg-muted rounded h-3 w-48"></div>
            </div>
            <div class="animate-pulse bg-muted rounded h-8 w-20"></div>
          </div>
        {/each}
      </div>
    {:else if apps.length === 0}
      <div class="px-4 py-10 text-center">
        <div class="w-12 h-12 mx-auto mb-3 rounded-full bg-muted flex items-center justify-center">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M14 10l-2 1m0 0l-2-1m2 1v2.5M20 7l-2 1m2-1l-2-1m2 1v2.5M14 4l-2-1-2 1M4 7l2-1M4 7l2 1M4 7v2.5M12 21l-2-1m2 1l2-1m-2 1v-2.5M6 18l-2-1v-2.5M18 18l2-1v-2.5" />
          </svg>
        </div>
        <p class="text-sm font-medium text-foreground mb-1">No apps installed</p>
        <p class="text-xs text-muted-foreground">Use the Apps tab to install WordPress, WooCommerce and more</p>
      </div>
    {:else}
      <div class="divide-y divide-border">
        {#each apps as app (app.id)}
          <div class="flex items-center gap-4 px-4 py-4">
            <!-- App icon with type-specific color -->
            <div class="w-10 h-10 rounded-lg flex items-center justify-center shrink-0 text-sm font-bold
              {app.app_id.toLowerCase().includes('wordpress') || app.app_id.toLowerCase().includes('wp')
                ? 'bg-blue-500/10 text-blue-400 border border-blue-500/20'
                : app.app_id.toLowerCase().includes('woocommerce')
                ? 'bg-purple-500/10 text-purple-400 border border-purple-500/20'
                : app.app_id.toLowerCase().includes('laravel')
                ? 'bg-red-500/10 text-red-400 border border-red-500/20'
                : 'bg-muted text-muted-foreground border border-border'}">
              {#if app.app_id.toLowerCase().includes('wordpress') || app.app_id.toLowerCase().includes('wp')}
                W
              {:else if app.app_id.toLowerCase().includes('woocommerce')}
                WC
              {:else if app.app_id.toLowerCase().includes('laravel')}
                L
              {:else}
                <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4" />
                </svg>
              {/if}
            </div>

            <!-- App info -->
            <div class="flex-1 min-w-0">
              <div class="flex items-center gap-2 flex-wrap">
                <span class="text-sm font-medium text-foreground">{app.app_id}</span>
                <span class="text-xs text-muted-foreground font-mono">v{app.version}</span>
                <!-- Status badge -->
                {#if app.status === 'active'}
                  <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">
                    Active
                  </span>
                {:else if app.status === 'updating'}
                  <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-yellow-500/10 text-yellow-400 border border-yellow-500/20">
                    Updating
                  </span>
                {:else if app.status === 'failed'}
                  <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-red-500/10 text-red-400 border border-red-500/20">
                    Failed
                  </span>
                {/if}
              </div>
              {#if app.admin_url}
                <a
                  href={app.admin_url}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="text-xs text-muted-foreground hover:text-primary transition-colors font-mono truncate block mt-0.5"
                >
                  {app.admin_url}
                </a>
              {:else}
                <p class="text-xs text-muted-foreground mt-0.5">Installed {app.installed_at ? formatDate(app.installed_at) : '—'}</p>
              {/if}
            </div>

            <!-- Update button placeholder — update detection not yet wired to API -->
            <!-- TODO: populate when api.apps.listInstalled returns update_available field -->
          </div>
        {/each}
      </div>
    {/if}
  </div>

</div>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  .card-hover { transition: all 0.2s ease; }
  .card-hover:hover { transform: translateY(-1px); box-shadow: 0 8px 30px rgba(0,0,0,0.2); }
</style>

<!-- ── Clone Site Modal ───────────────────────────────────────────────────────── -->
{#if showCloneModal}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={() => { if (!cloneLoading) showCloneModal = false; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <div>
          <h3 class="text-base font-semibold text-foreground">Clone Site</h3>
          <p class="text-xs text-muted-foreground mt-0.5">Copies files from <span class="font-mono">{site.domain}</span></p>
        </div>
        <button
          class="text-muted-foreground hover:text-foreground transition-colors"
          disabled={cloneLoading}
          on:click={() => showCloneModal = false}
        >
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      {#if cloneResult}
        <div class="bg-green-500/10 border border-green-500/20 rounded-xl p-4 mb-4">
          <p class="text-sm text-green-400">{cloneResult}</p>
        </div>
        <button
          class="w-full h-9 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted"
          on:click={() => showCloneModal = false}
        >
          Close
        </button>
      {:else}
        <div class="space-y-4">
          <div>
            <label class="block text-xs font-medium text-muted-foreground mb-1">Target domain</label>
            <input
              type="text"
              bind:value={cloneTargetDomain}
              placeholder="clone.example.com"
              class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
            />
          </div>

          <label class="flex items-center gap-3 p-3 rounded-lg border border-border bg-muted/20 cursor-pointer">
            <input type="checkbox" bind:checked={cloneCopyDb} class="rounded accent-primary" />
            <div>
              <span class="text-sm font-medium text-foreground">Copy database</span>
              <p class="text-xs text-muted-foreground">Clone the MySQL database and update WordPress URLs</p>
            </div>
          </label>

          {#if cloneError}
            <p class="text-sm text-red-400">{cloneError}</p>
          {/if}

          <div class="flex gap-2 pt-1">
            <button
              class="flex-1 h-9 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted"
              disabled={cloneLoading}
              on:click={() => showCloneModal = false}
            >
              Cancel
            </button>
            <button
              class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 disabled:opacity-50 inline-flex items-center justify-center gap-2"
              disabled={cloneLoading || !cloneTargetDomain.trim()}
              on:click={cloneSite}
            >
              {#if cloneLoading}
                <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                </svg>
                Cloning…
              {:else}
                Clone Site
              {/if}
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}


<!-- ── Toast ──────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div
    class="fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border transition-all duration-300
      {toast.type === 'success'
        ? 'bg-green-950/90 border-green-800 text-green-300'
        : 'bg-red-950/90 border-red-800 text-red-300'}"
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
    <button
      on:click={() => { toast = null; }}
      class="text-current opacity-60 hover:opacity-100 transition-opacity"
      aria-label="Dismiss"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>
{/if}
