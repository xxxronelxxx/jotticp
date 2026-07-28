<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { api } from '$api/client';
  import type { Server, Site } from '$api/client';

  // ── State ──────────────────────────────────────────────────────────────────
  let server: Server | null = null;
  let sites: Site[] = [];
  let loading = true;
  let loadError = '';
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let testing = false;
  let confirmDelete = false;
  let deleting = false;
  let metricsInterval: ReturnType<typeof setInterval> | null = null;

  $: serverId = $page.params.id;

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await loadData();
    // Poll metrics every 10s if server is active
    metricsInterval = setInterval(async () => {
      if (server?.status === 'active') await refreshMetrics();
    }, 10_000);
  });

  onDestroy(() => {
    if (metricsInterval) clearInterval(metricsInterval);
  });

  async function loadData() {
    loading = true;
    loadError = '';
    try {
      const [srv, allSites] = await Promise.all([
        api.servers.get(serverId),
        api.sites.list(),
      ]);
      server = srv;
      sites = allSites.filter(s => s.server_id === serverId);
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load server';
    } finally {
      loading = false;
    }
  }

  async function refreshMetrics() {
    try {
      const updated = await api.servers.get(serverId);
      if (server) {
        server = { ...server, ...updated };
      }
    } catch { /* silent */ }
  }

  // ── Actions ────────────────────────────────────────────────────────────────
  async function testConnection() {
    if (!server) return;
    testing = true;
    try {
      const result = await api.servers.testConnection(server.id);
      showToast(
        result.success ? 'Connection successful' : `Failed: ${result.message}`,
        result.success ? 'success' : 'error'
      );
    } catch {
      showToast('Connection test failed', 'error');
    } finally {
      testing = false;
    }
  }

  async function deleteServer() {
    if (!server) return;
    deleting = true;
    try {
      await api.servers.delete(server.id);
      showToast('Server removed', 'success');
      setTimeout(() => goto('/servers'), 1200);
    } catch {
      showToast('Failed to remove server', 'error');
      deleting = false;
    }
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  // ── Helpers ────────────────────────────────────────────────────────────────
  function pct(val: number | undefined): number {
    if (val === undefined || val === null) return 0;
    return Math.min(100, Math.max(0, val));
  }

  function pctColor(val: number | undefined): string {
    const v = val ?? 0;
    if (v >= 85) return 'bg-red-400';
    if (v >= 60) return 'bg-amber-400';
    return 'bg-primary';
  }

  function pctText(val: number | undefined): string {
    const v = val ?? 0;
    if (v >= 85) return 'text-red-400';
    if (v >= 60) return 'text-amber-400';
    return 'text-foreground';
  }

  function formatRam(mb: number | null): string {
    if (!mb) return '—';
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
  }

  function formatDisk(gb: number | null): string {
    if (!gb) return '—';
    return gb >= 1024 ? `${(gb / 1024).toFixed(1)} TB` : `${gb} GB`;
  }

  function formatUptime(sec: number | null | undefined): string {
    if (!sec) return '—';
    const d = Math.floor(sec / 86400);
    const h = Math.floor((sec % 86400) / 3600);
    const m = Math.floor((sec % 3600) / 60);
    if (d > 0) return `${d}d ${h}h`;
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  }

  function statusColor(status: string): string {
    if (status === 'active') return 'text-green-400';
    if (status === 'offline') return 'text-red-400';
    if (status === 'pending_enrollment') return 'text-amber-400';
    return 'text-muted-foreground';
  }

  function statusDot(status: string): string {
    if (status === 'active') return 'bg-green-400';
    if (status === 'offline') return 'bg-red-400';
    if (status === 'pending_enrollment') return 'bg-amber-400';
    return 'bg-muted-foreground';
  }

  function statusLabel(status: string): string {
    if (status === 'active') return 'Online';
    if (status === 'offline') return 'Offline';
    if (status === 'pending_enrollment') return 'Pending Enrollment';
    if (status === 'error') return 'Error';
    return status;
  }

  function siteStatusColor(s: Site): string {
    if (s.status === 'active') return 'text-green-400 bg-green-500/10 border-green-500/20';
    if (s.status === 'error') return 'text-red-400 bg-red-500/10 border-red-500/20';
    return 'text-amber-400 bg-amber-500/10 border-amber-500/20';
  }

  function formatDate(d: string): string {
    return new Date(d).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric' });
  }
</script>

<svelte:head>
  <title>{server?.label ?? 'Server'} — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-6">

  <!-- ── Back nav ──────────────────────────────────────────────────────────── -->
  <a href="/servers"
     class="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors">
    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
    </svg>
    Back to Servers
  </a>

  <!-- ── Load error ────────────────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      {loadError}
      <button on:click={() => void loadData()} class="ml-auto text-xs underline">Retry</button>
    </div>

  <!-- ── Skeleton ──────────────────────────────────────────────────────────── -->
  {:else if loading}
    <div class="space-y-4 animate-pulse">
      <div class="h-8 w-48 rounded-lg bg-muted"></div>
      <div class="grid grid-cols-2 lg:grid-cols-4 gap-4">
        {#each [1,2,3,4] as _}
          <div class="h-24 rounded-xl bg-muted"></div>
        {/each}
      </div>
      <div class="h-48 rounded-xl bg-muted"></div>
    </div>

  {:else if server}
    <!-- ── Header ──────────────────────────────────────────────────────────── -->
    <div class="flex flex-col sm:flex-row sm:items-start justify-between gap-4 fade-up">
      <div class="flex items-start gap-4">
        <!-- Icon -->
        <div class="w-12 h-12 rounded-xl bg-primary/10 border border-primary/20 flex items-center justify-center shrink-0">
          <svg class="w-6 h-6 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M5 3h14a2 2 0 012 2v4a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2zM5 13h14a2
                 2 0 012 2v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4a2 2 0 012-2zM9 7h.01M9 17h.01" />
          </svg>
        </div>
        <div>
          <h1 class="text-2xl font-bold text-foreground">{server.label}</h1>
          <div class="flex items-center gap-3 mt-1 flex-wrap">
            <!-- Status -->
            <span class="flex items-center gap-1.5 text-sm {statusColor(server.status)} font-medium">
              <span class="relative flex h-2 w-2">
                <span class="w-2 h-2 rounded-full block {statusDot(server.status)}"></span>
                {#if server.status === 'active'}
                  <span class="absolute inset-0 rounded-full bg-green-400 animate-ping opacity-50"></span>
                {/if}
              </span>
              {statusLabel(server.status)}
            </span>
            <!-- IP -->
            <span class="font-mono text-sm text-muted-foreground">{server.ip}</span>
            {#if server.os_version}
              <span class="px-2 py-0.5 rounded-full text-xs font-medium bg-muted text-muted-foreground">
                {server.os_version}
              </span>
            {/if}
            {#if server.last_seen_at}
              <span class="text-xs text-muted-foreground">Last seen {formatDate(server.last_seen_at)}</span>
            {/if}
          </div>
        </div>
      </div>

      <!-- Action buttons -->
      <div class="flex items-center gap-2 shrink-0">
        <button
          on:click={testConnection}
          disabled={testing}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2
                 transition-all duration-150 active:scale-95 disabled:opacity-50"
        >
          {#if testing}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            Testing…
          {:else}
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
            Test Connection
          {/if}
        </button>

        {#if confirmDelete}
          <span class="text-sm text-destructive">Remove server?</span>
          <button
            on:click={deleteServer}
            disabled={deleting}
            class="h-9 px-4 rounded-lg bg-destructive text-white text-sm font-medium
                   hover:bg-destructive/90 transition-all active:scale-95 disabled:opacity-50"
          >
            {deleting ? 'Removing…' : 'Yes, remove'}
          </button>
          <button
            on:click={() => confirmDelete = false}
            class="h-9 px-3 rounded-lg border border-border text-sm hover:bg-muted transition-all"
          >
            Cancel
          </button>
        {:else}
          <button
            on:click={() => confirmDelete = true}
            title="Remove server"
            class="h-9 w-9 rounded-lg border border-border text-muted-foreground
                   hover:bg-red-500/10 hover:text-red-400 hover:border-red-500/20
                   inline-flex items-center justify-center transition-all duration-150 active:scale-95"
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
            </svg>
          </button>
        {/if}
      </div>
    </div>

    <!-- ── Pending enrollment banner ──────────────────────────────────────── -->
    {#if server.status === 'pending_enrollment'}
      <div class="bg-amber-500/10 border border-amber-500/20 rounded-xl p-4 fade-up-1">
        <div class="flex items-start gap-3">
          <svg class="w-5 h-5 text-amber-400 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <div class="flex-1 min-w-0">
            <p class="text-sm font-semibold text-amber-400 mb-1">Server not enrolled yet</p>
            <p class="text-xs text-muted-foreground mb-3">
              SSH into <span class="font-mono text-foreground">{server.ip}</span> and run the orbit-agent enrollment command to connect this server to JottiCP.
            </p>
            <div class="bg-black/30 rounded-lg px-3 py-2 font-mono text-xs text-green-300 break-all">
              curl -s https://jottiecp.dev-spb.ru/api/v1/servers/{server.id}/enroll | bash
            </div>
            <p class="text-xs text-muted-foreground mt-2">
              Once enrolled, metrics and site management will be available.
            </p>
          </div>
        </div>
      </div>
    {/if}

    <!-- ── Metric cards ────────────────────────────────────────────────────── -->
    <div class="grid grid-cols-2 lg:grid-cols-4 gap-4 fade-up-1">
      {#each [
        { label: 'CPU',     val: server.cpu_pct,   display: server.cpu_pct != null  ? `${server.cpu_pct.toFixed(1)}%`  : '—', icon: 'M9 3H5a2 2 0 00-2 2v4m6-6h10a2 2 0 012 2v4M9 3v18m0 0h10a2 2 0 002-2V9M9 21H5a2 2 0 01-2-2V9m0 0h18' },
        { label: 'RAM',     val: server.ram_pct,   display: server.ram_pct != null  ? `${server.ram_pct.toFixed(1)}%`  : '—', icon: 'M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4' },
        { label: 'Disk',    val: server.disk_pct,  display: server.disk_pct != null ? `${server.disk_pct.toFixed(1)}%` : '—', icon: 'M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4' },
        { label: 'Uptime',  val: undefined,        display: formatUptime(server.uptime_seconds),                              icon: 'M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z' },
      ] as card}
        <div class="bg-card border border-border rounded-xl p-4 space-y-3 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-lg hover:shadow-black/20">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-2">
              <svg class="w-4 h-4 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d={card.icon} />
              </svg>
              <span class="text-xs text-muted-foreground uppercase tracking-wider font-medium">{card.label}</span>
            </div>
            <span class="text-lg font-bold {card.val !== undefined ? pctText(card.val) : 'text-foreground'}">
              {card.display}
            </span>
          </div>
          {#if card.val !== undefined}
            <div class="h-1.5 bg-primary/20 rounded-full overflow-hidden">
              <div class="h-full rounded-full transition-all duration-700 {pctColor(card.val)}"
                   style="width: {pct(card.val)}%"></div>
            </div>
          {/if}
        </div>
      {/each}
    </div>

    <!-- ── Info grid ─────────────────────────────────────────────────────────── -->
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 fade-up-2">

      <!-- Server info -->
      <div class="bg-card border border-border rounded-xl p-5">
        <h2 class="text-sm font-semibold text-foreground mb-4">Server Details</h2>
        <dl class="space-y-3">
          {#each [
            { label: 'Label',       value: server.label },
            { label: 'IP Address',  value: server.ip,           mono: true },
            { label: 'OS',          value: server.os_version ?? '—' },
            { label: 'CPU Cores',   value: server.cpu_count     != null ? `${server.cpu_count} cores` : '—' },
            { label: 'Total RAM',   value: formatRam(server.ram_total_mb) },
            { label: 'Total Disk',  value: formatDisk(server.disk_total_gb) },
            { label: 'Load (1m)',   value: server.load_1        != null ? server.load_1.toFixed(2) : '—' },
            { label: 'Sites',       value: `${sites.length}` },
            { label: 'Added',       value: formatDate(server.created_at) },
          ] as row}
            <div class="flex items-center justify-between gap-2">
              <dt class="text-xs text-muted-foreground">{row.label}</dt>
              <dd class="text-sm font-medium {row.mono ? 'font-mono' : ''} text-foreground truncate max-w-[60%] text-right">
                {row.value}
              </dd>
            </div>
          {/each}
        </dl>
      </div>

      <!-- Sites on this server -->
      <div class="bg-card border border-border rounded-xl p-5">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-sm font-semibold text-foreground">Sites on this Server</h2>
          <span class="text-xs text-muted-foreground">{sites.length} site{sites.length !== 1 ? 's' : ''}</span>
        </div>

        {#if sites.length === 0}
          <div class="text-center py-8">
            <svg class="w-8 h-8 mx-auto text-muted-foreground mb-2" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9" />
            </svg>
            <p class="text-xs text-muted-foreground">No sites assigned to this server</p>
            <a href="/websites" class="text-xs text-primary hover:underline mt-1 inline-block">Manage sites →</a>
          </div>
        {:else}
          <ul class="space-y-2">
            {#each sites as site}
              <li>
                <a
                  href="/websites/{site.id}"
                  class="flex items-center justify-between gap-2 p-3 rounded-lg
                         hover:bg-muted/50 transition-colors duration-150 group"
                >
                  <div class="flex items-center gap-2 min-w-0">
                    <svg class="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9" />
                    </svg>
                    <span class="text-sm text-foreground truncate font-medium group-hover:text-primary transition-colors">
                      {site.domain}
                    </span>
                  </div>
                  <span class="text-[10px] font-medium px-1.5 py-0.5 rounded-full border shrink-0 {siteStatusColor(site)}">
                    {site.status}
                  </span>
                </a>
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    </div>

    <!-- ── Quick links ───────────────────────────────────────────────────────── -->
    <div class="grid grid-cols-2 sm:grid-cols-4 gap-3 fade-up-2">
      {#each [
        { href: `/logs?server=${server.id}`,        label: 'Error Logs',    icon: 'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2' },
        { href: `/filemanager?server=${server.id}`, label: 'File Manager',  icon: 'M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z' },
        { href: '/firewall',                        label: 'Firewall',      icon: 'M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z' },
        { href: '/backups',                         label: 'Backups',       icon: 'M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4' },
      ] as link}
        <a
          href={link.href}
          class="flex items-center gap-2.5 p-3 rounded-xl border border-border bg-card
                 hover:bg-muted hover:border-primary/20 transition-all duration-150 group"
        >
          <svg class="w-4 h-4 text-muted-foreground group-hover:text-primary transition-colors shrink-0"
               fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5" d={link.icon} />
          </svg>
          <span class="text-sm text-muted-foreground group-hover:text-foreground transition-colors font-medium">
            {link.label}
          </span>
        </a>
      {/each}
    </div>
  {/if}
</div>

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div
    class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm
           shadow-lg flex items-center gap-2 fade-up"
    role="status"
    aria-live="polite"
  >
    {#if toastType === 'success'}
      <svg class="w-4 h-4 shrink-0 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
      </svg>
    {:else}
      <svg class="w-4 h-4 shrink-0 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
      </svg>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px) }
    to   { opacity: 1; transform: none }
  }
  .fade-up   { animation: fadeUp 0.25s ease-out both }
  .fade-up-1 { animation: fadeUp 0.25s 0.05s ease-out both }
  .fade-up-2 { animation: fadeUp 0.25s 0.1s ease-out both }
</style>
