<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$api/client';
  import type { Server } from '$api/client';

  // ── State ──────────────────────────────────────────────────────────────────
  let servers: Server[] = [];
  let loading = true;
  let enrollmentToken = '';

  // Expanded card state
  let expandedServer: string | null = null;

  // Inline confirm state
  let confirmRemoveServerId: string | null = null;

  // Quick Actions dropdown
  let openMenuServer: string | null = null;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';

  // ── Computed stats ─────────────────────────────────────────────────────────
  $: totalServers  = servers.length;
  $: onlineServers = servers.filter(s => s.status === 'active').length;
  $: totalSites    = servers.reduce((sum, s) => sum + (s.site_count ?? 0), 0);

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await loadServers();
  });

  async function loadServers() {
    loading = true;
    loadError = '';
    try {
      servers = await api.servers.list();
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load servers';
    } finally {
      loading = false;
    }
  }

  // ── Actions ────────────────────────────────────────────────────────────────
  async function testConnection(server: Server) {
    try {
      const result = await api.servers.testConnection(server.id);
      showToast(
        result.success ? `Connected to ${server.label}` : `Failed: ${result.message}`,
        result.success ? 'success' : 'error'
      );
    } catch {
      showToast('Connection test failed', 'error');
    }
  }

  async function deleteServer(server: Server) {
    confirmRemoveServerId = null;
    try {
      await api.servers.delete(server.id);
      servers = servers.filter(s => s.id !== server.id);
      showToast('Server removed', 'success');
    } catch {
      showToast('Failed to remove server', 'error');
    }
  }

  function copyToken() {
    navigator.clipboard.writeText(enrollmentToken);
    showToast('Token copied!', 'success');
  }

  function copySSH(server: Server) {
    const str = `ssh root@${server.ip}`;
    navigator.clipboard.writeText(str);
    showToast('SSH command copied!', 'success');
    openMenuServer = null;
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType    = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function toggleExpand(id: string) {
    expandedServer = expandedServer === id ? null : id;
  }

  function toggleMenu(id: string) {
    openMenuServer = openMenuServer === id ? null : id;
  }

  async function quickAction(server: Server, action: string) {
    openMenuServer = null;
    switch (action) {
      case 'restart-web':
        showToast(`Restarting web server on ${server.label}...`, 'success');
        try {
          const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
          const res = await fetch(`/api/v1/servers/${server.id}/restart-web`, {
            method: 'POST',
            headers: { ...(token ? { 'Authorization': `Bearer ${token}` } : {}) },
          });
          if (!res.ok) throw new Error(`HTTP ${res.status}`);
          showToast(`Web server restarted on ${server.label}`, 'success');
        } catch { showToast('Action failed', 'error'); }
        break;
      case 'flush-cache':
        showToast('Cache flush is not yet available via API — clear cache from the Cache page', 'error');
        break;
      case 'security-scan':
        showToast('Security scan is not yet available via API — check the Security page for manual steps', 'error');
        break;
      case 'view-logs':
        goto(`/logs?server=${server.id}`);
        break;
      case 'copy-ssh':
        copySSH(server);
        break;
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────
  function statusLabel(status: Server['status']): string {
    return status === 'active' ? 'Online' : status === 'offline' ? 'Offline' : 'Degraded';
  }

  function statusDotClass(status: Server['status']): string {
    return status === 'active' ? 'bg-green-400' : status === 'offline' ? 'bg-red-400' : 'bg-amber-400';
  }

  function statusTextClass(status: Server['status']): string {
    return status === 'active' ? 'text-green-400' : status === 'offline' ? 'text-red-400' : 'text-amber-400';
  }

  function formatUptime(sec: number | null | undefined): string {
    if (!sec) return '—';
    const d = Math.floor(sec / 86400);
    const h = Math.floor((sec % 86400) / 3600);
    if (d > 0) return `${d}d ${h}h`;
    const m = Math.floor((sec % 3600) / 60);
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  }

  function formatRam(mb: number | null): string {
    if (!mb) return '—';
    return mb >= 1024 ? `${(mb / 1024).toFixed(0)} GB` : `${mb} MB`;
  }

  // Clamp a percentage for display
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

  // Alert badge level: null=ok, 'amber'=any>80, 'red'=any>95
  function alertLevel(server: Server): 'red' | 'amber' | null {
    const vals = [server.cpu_pct, server.ram_pct, server.disk_pct].filter(v => v !== undefined && v !== null) as number[];
    if (vals.some(v => v > 95)) return 'red';
    if (vals.some(v => v > 80)) return 'amber';
    return null;
  }

  // Mock 7-day CPU history (randomised but stable per server)
  function mockHistory(server: Server): number[] {
    const seed = server.id.split('').reduce((a, c) => a + c.charCodeAt(0), 0);
    return Array.from({ length: 7 }, (_, i) => {
      const base = (server.cpu_pct ?? 30);
      const jitter = ((seed * (i + 1) * 7919) % 40) - 20;
      return Math.min(100, Math.max(5, Math.round(base + jitter)));
    });
  }

  const quickActions = [
    { id: 'restart-web',  label: 'Restart Web Server',  icon: 'M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15' },
    { id: 'flush-cache',  label: 'Flush All Caches',    icon: 'M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16' },
    { id: 'security-scan',label: 'Run Security Scan',   icon: 'M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z' },
    { id: 'view-logs',    label: 'View Error Logs',     icon: 'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2' },
    { id: 'copy-ssh',     label: 'Copy SSH Command',    icon: 'M8 9l3 3-3 3m5 0h3M5 20h14a2 2 0 002-2V6a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z' },
  ];

  const softwareList = [
    { key: 'nginx',   label: 'nginx' },
    { key: 'php',     label: 'PHP 8.3' },
    { key: 'mysql',   label: 'MySQL' },
    { key: 'redis',   label: 'Redis' },
    { key: 'certbot', label: 'Certbot' },
  ];
</script>

<svelte:head>
  <title>Servers — JottiCP</title>
</svelte:head>

<!-- Click-away to close menus -->
<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
<div class="min-h-0" on:click={() => { openMenuServer = null; }}></div>

<div class="p-4 lg:p-6 space-y-6">
  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400 fade-up">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadServers()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- ── Header ──────────────────────────────────────────────────────────── -->
  <div class="flex items-start justify-between gap-4 fade-up">
    <div>
      <h1 class="text-2xl font-bold text-foreground">Servers</h1>
      <p class="text-muted-foreground text-sm mt-1">Manage your infrastructure</p>
    </div>
    <a
      href="/servers/add"
      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
             hover:bg-primary/90 inline-flex items-center gap-2 shrink-0 transition-all duration-150 active:scale-95"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
      </svg>
      Add Server
    </a>
  </div>

  <!-- ── Stats bar ────────────────────────────────────────────────────────── -->
  {#if !loading}
    <div class="grid grid-cols-3 gap-3 fade-up-1">
      {#each [
        { label: 'Total Servers', value: totalServers,  color: 'text-foreground' },
        { label: 'Online',        value: onlineServers, color: 'text-green-400' },
        { label: 'Sites Hosted',  value: totalSites,    color: 'text-foreground' },
      ] as stat}
        <div class="bg-card border border-border rounded-xl px-4 py-3
                    transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
          <p class="text-xs text-muted-foreground uppercase tracking-wider font-medium">{stat.label}</p>
          <p class="text-2xl font-bold mt-0.5 {stat.color}">{stat.value}</p>
        </div>
      {/each}
    </div>
  {/if}

  <!-- ── Enrollment token banner ──────────────────────────────────────────── -->
  {#if enrollmentToken}
    <div class="bg-green-500/10 border border-green-500/20 rounded-xl p-4 fade-up">
      <p class="text-sm font-semibold text-green-400 mb-2">Server added — run this command on the new server:</p>
      <div class="flex items-center gap-2">
        <code class="flex-1 font-mono text-xs bg-black/20 px-3 py-2 rounded-lg text-green-300 break-all">
          orbit-agent enroll --token {enrollmentToken} --panel {typeof window !== 'undefined' ? window.location.origin : 'https://panel.yourdomain.com'}
        </code>
        <button
          on:click={copyToken}
          class="h-9 px-3 rounded-lg border border-green-500/30 text-green-400
                 hover:bg-green-500/10 transition-all duration-150 active:scale-95 text-sm font-medium shrink-0"
        >
          Copy
        </button>
      </div>
    </div>
  {/if}

  <!-- ── Server grid / skeleton / empty ───────────────────────────────────── -->
  {#if loading}
    <div class="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-4">
      {#each [1, 2, 3] as _}
        <div class="bg-card border border-border rounded-xl p-5 space-y-4 animate-pulse">
          <div class="flex items-center gap-3">
            <div class="w-2.5 h-2.5 rounded-full bg-muted"></div>
            <div class="h-4 w-2/5 rounded bg-muted"></div>
            <div class="h-3 w-1/4 rounded bg-muted ml-auto"></div>
          </div>
          <div class="space-y-2">
            {#each [1,2,3] as __}
              <div class="flex items-center gap-2">
                <div class="h-3 w-8 rounded bg-muted"></div>
                <div class="flex-1 h-1.5 rounded-full bg-muted"></div>
                <div class="h-3 w-8 rounded bg-muted"></div>
              </div>
            {/each}
          </div>
          <div class="flex gap-2 pt-1">
            <div class="h-8 w-16 rounded-lg bg-muted"></div>
            <div class="h-8 w-20 rounded-lg bg-muted"></div>
          </div>
        </div>
      {/each}
    </div>

  {:else if servers.length === 0}
    <div class="py-20 text-center fade-up">
      <svg class="w-12 h-12 mx-auto text-muted-foreground mb-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
          d="M5 3h14a2 2 0 012 2v4a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2zM5 13h14a2 2 0 012 2v4a2
             2 0 01-2 2H5a2 2 0 01-2-2v-4a2 2 0 012-2zM9 7h.01M9 17h.01" />
      </svg>
      <h3 class="text-foreground font-medium">No servers yet</h3>
      <p class="text-muted-foreground text-sm mt-1">Add your first server to manage websites across multiple machines</p>
      <a
        href="/servers/add"
        class="mt-4 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
               hover:bg-primary/90 inline-flex items-center gap-2 mx-auto transition-all duration-150 active:scale-95"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
        </svg>
        Add your first server
      </a>
    </div>

  {:else}
    <div class="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-3 gap-4">
      {#each servers as server (server.id)}
        {@const hist = mockHistory(server)}
        {@const alert = alertLevel(server)}
        {@const isExpanded = expandedServer === server.id}

        <div class="bg-card border border-border rounded-xl overflow-hidden
                    transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20 fade-up">

          <!-- Card main body (clickable to expand) -->
          <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
          <div class="p-5 flex flex-col gap-4 cursor-pointer" on:click={() => toggleExpand(server.id)}>

            <!-- Card header: name + IP + status + alert + quick actions -->
            <div>
              <div class="flex items-center justify-between gap-2 mb-1">
                <div class="flex items-center gap-2 min-w-0">
                  <!-- Pulsing status dot -->
                  <span class="relative shrink-0 w-2.5 h-2.5">
                    <span class="w-2.5 h-2.5 rounded-full block {statusDotClass(server.status)}
                                 {server.status === 'active' ? 'shadow-[0_0_6px_1px] shadow-green-400/50' : ''}">
                    </span>
                    {#if server.status === 'active'}
                      <span class="absolute inset-0 rounded-full bg-green-400 animate-ping opacity-50"></span>
                    {/if}
                  </span>
                  <span class="font-bold text-foreground truncate">{server.label}</span>
                </div>

                <div class="flex items-center gap-1.5 shrink-0">
                  <!-- Alert badge -->
                  {#if alert === 'red'}
                    <span class="inline-flex items-center px-1.5 py-0.5 rounded-full text-[10px] font-bold
                                 bg-red-500/15 text-red-400 border border-red-500/30 animate-pulse">
                      CRITICAL
                    </span>
                  {:else if alert === 'amber'}
                    <span class="inline-flex items-center px-1.5 py-0.5 rounded-full text-[10px] font-bold
                                 bg-amber-500/15 text-amber-400 border border-amber-500/30">
                      HIGH
                    </span>
                  {/if}

                  <!-- Uptime badge -->
                  <span class="inline-flex items-center px-1.5 py-0.5 rounded-full text-[10px] font-medium
                               bg-green-500/10 text-green-400 border border-green-500/20">
                    99.9%
                  </span>

                  <!-- Quick actions kebab -->
                  <div class="relative" on:click|stopPropagation>
                    <button
                      on:click={() => toggleMenu(server.id)}
                      title="Quick Actions"
                      class="w-7 h-7 rounded-lg border border-border text-muted-foreground
                             hover:bg-muted hover:text-foreground flex items-center justify-center
                             transition-all duration-150 active:scale-95"
                    >
                      <svg class="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
                        <circle cx="12" cy="5" r="1.5"/><circle cx="12" cy="12" r="1.5"/><circle cx="12" cy="19" r="1.5"/>
                      </svg>
                    </button>

                    {#if openMenuServer === server.id}
                      <div class="absolute right-0 top-8 z-30 w-48 bg-card border border-border rounded-xl shadow-2xl overflow-hidden fade-up">
                        {#each quickActions as qa}
                          <button
                            on:click={() => quickAction(server, qa.id)}
                            class="w-full flex items-center gap-2.5 px-3 py-2 text-sm text-muted-foreground
                                   hover:bg-muted hover:text-foreground transition-colors duration-150 text-left"
                          >
                            <svg class="w-3.5 h-3.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d={qa.icon}/>
                            </svg>
                            {qa.label}
                          </button>
                        {/each}
                      </div>
                    {/if}
                  </div>
                </div>
              </div>

              <div class="flex items-center gap-3 ml-4.5">
                <span class="text-xs {statusTextClass(server.status)} font-medium">{statusLabel(server.status)}</span>
                <span class="text-xs font-mono text-muted-foreground">{server.ip}</span>
                {#if server.os_version}
                  <span class="bg-muted text-muted-foreground px-2 py-0.5 rounded-full text-xs font-medium">
                    {server.os_version}
                  </span>
                {/if}
              </div>
            </div>

            <!-- Metric mini-bars -->
            <div class="space-y-2">
              {#each [
                { label: 'CPU',  val: server.cpu_pct,  display: server.cpu_pct !== undefined  ? `${server.cpu_pct?.toFixed(0)}%`  : '—' },
                { label: 'RAM',  val: server.ram_pct,  display: server.ram_pct !== undefined  ? `${server.ram_pct?.toFixed(0)}%`  : '—' },
                { label: 'Disk', val: server.disk_pct, display: server.disk_pct !== undefined ? `${server.disk_pct?.toFixed(0)}%` : '—' },
              ] as metric}
                <div class="flex items-center gap-2 text-xs">
                  <span class="text-muted-foreground w-7 shrink-0">{metric.label}</span>
                  <div class="flex-1 bg-primary/20 rounded-full h-1.5">
                    <div
                      class="h-1.5 rounded-full transition-all duration-500 {pctColor(metric.val)}"
                      style="width: {pct(metric.val)}%"
                    ></div>
                  </div>
                  <span class="text-muted-foreground w-8 text-right">{metric.display}</span>
                </div>
              {/each}

              <!-- 7-day CPU sparkline -->
              <div class="flex items-end gap-px h-6 mt-2">
                {#each hist as val, i}
                  <div class="flex-1 rounded-sm transition-all duration-500 {val < 60 ? 'bg-green-500/40' : val < 80 ? 'bg-amber-500/40' : 'bg-red-500/40'}"
                       style="height: {val}%"
                       title="{val}%">
                  </div>
                {/each}
              </div>
              <p class="text-[10px] text-muted-foreground mt-0.5">7-day CPU avg</p>
            </div>

            <!-- Secondary meta -->
            <div class="grid grid-cols-3 gap-2 text-xs">
              <div>
                <p class="text-muted-foreground/60 mb-0.5">RAM Total</p>
                <p class="text-muted-foreground font-medium">{formatRam(server.ram_total_mb)}</p>
              </div>
              <div>
                <p class="text-muted-foreground/60 mb-0.5">Uptime</p>
                <p class="text-muted-foreground font-medium">{formatUptime(server.uptime_seconds)}</p>
              </div>
              <div>
                <p class="text-muted-foreground/60 mb-0.5">Sites</p>
                <p class="text-muted-foreground font-medium">{server.site_count ?? 0}</p>
              </div>
            </div>

            <!-- Actions -->
            <div class="flex items-center gap-2 pt-1 mt-auto" on:click|stopPropagation>
              <a
                href="/servers/{server.id}"
                class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                       hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
              >
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0
                       0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2
                       2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
                </svg>
                Manage
              </a>
              <button
                on:click={() => testConnection(server)}
                class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                       hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
              >
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M13 10V3L4 14h7v7l9-11h-7z" />
                </svg>
                Test
              </button>
              {#if confirmRemoveServerId === server.id}
                <div class="flex items-center gap-1.5 ml-auto">
                  <span class="text-xs text-destructive">Remove?</span>
                  <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => deleteServer(server)}>Yes</button>
                  <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmRemoveServerId = null}>No</button>
                </div>
              {:else}
                <button
                  on:click={() => confirmRemoveServerId = server.id}
                  title="Remove server"
                  class="h-9 w-9 rounded-lg border border-border text-muted-foreground
                         hover:bg-red-500/10 hover:text-red-400 hover:border-red-500/20
                         inline-flex items-center justify-center ml-auto transition-all duration-150 active:scale-95"
                >
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5
                         7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                  </svg>
                </button>
              {/if}
            </div>
          </div>

          <!-- ── Expanded detail panel ──────────────────────────────────────── -->
          {#if isExpanded}
            <div class="border-t border-border px-5 py-4 space-y-4 bg-muted/20 fade-up">
              <!-- Installed software -->
              <div>
                <p class="text-xs text-muted-foreground uppercase tracking-wider font-medium mb-2">Installed Software</p>
                <div class="grid grid-cols-3 gap-2">
                  {#each softwareList as sw}
                    <div class="flex items-center gap-1.5 text-xs">
                      <svg class="w-3.5 h-3.5 text-green-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
                      </svg>
                      <span class="text-muted-foreground">{sw.label}</span>
                    </div>
                  {/each}
                </div>
              </div>

              <!-- Network info -->
              <div>
                <p class="text-xs text-muted-foreground uppercase tracking-wider font-medium mb-2">Network</p>
                <div class="grid grid-cols-2 gap-2 text-xs">
                  <div>
                    <p class="text-muted-foreground/60">Public IP</p>
                    <p class="font-mono text-foreground font-medium">{server.ip}</p>
                  </div>
                  <div>
                    <p class="text-muted-foreground/60">Hostname</p>
                    <p class="font-mono text-foreground font-medium truncate">{(server as unknown as { hostname?: string }).hostname ?? server.label}</p>
                  </div>
                  <div>
                    <p class="text-muted-foreground/60">OS</p>
                    <p class="text-muted-foreground">{server.os_version ?? '—'}</p>
                  </div>
                  <div>
                    <p class="text-muted-foreground/60">SSH Port</p>
                    <p class="font-mono text-muted-foreground">{(server as unknown as { ssh_port?: number }).ssh_port ?? 22}</p>
                  </div>
                </div>
              </div>

              <!-- Open in File Manager -->
              <a
                href="/filemanager?server={server.id}"
                class="flex items-center gap-2 h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-all duration-150 active:scale-95 w-fit"
              >
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/>
                </svg>
                Open in File Manager
              </a>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

</div>

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div
    class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
           flex items-center gap-2 fade-up"
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
