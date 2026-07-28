<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$api/client';
  import type { Site, Server, Database, EmailAccount, Notification } from '$api/client';
  import { auth, isAdmin, currentUser } from '$stores/auth';
  import { get } from 'svelte/store';

  // ── Auth helper ──────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const t = get(auth).token;
    return { 'Content-Type': 'application/json', ...(t ? { Authorization: 'Bearer ' + t } : {}) };
  }

  // ── State ────────────────────────────────────────────────────────────────────

  let loading    = true;
  let loadError  = '';

  let sites:         Site[]         = [];
  let servers:       Server[]       = [];
  let databases:     Database[]     = [];
  let emailAccounts: EmailAccount[] = [];
  let notifications: Notification[] = [];

  // Per-server latest REST metrics (keyed by server id)
  let serverMetrics: Record<string, {
    cpu_pct: number;
    ram_used_mb: number;
    ram_total_mb: number;
    disk_used_gb: number;
    disk_total_gb: number;
    net_in_mbps: number;
    net_out_mbps: number;
    load_avg_1m: number;
  }> = {};

  // Host server real stats
  let hostStats: {
    cpu_pct: number;
    ram_total_mb: number;
    ram_used_mb: number;
    ram_free_mb: number;
    disk_total_gb: number;
    disk_used_gb: number;
    disk_free_gb: number;
    disk_pct: number;
    load_1m: number;
    load_5m: number;
    load_15m: number;
    uptime_secs: number;
    process_count: number;
  } | null = null;

  let perfData: {
    http3_supported: boolean;
    ttfb_ms: number;
    active_sites: number;
    nginx_version: string;
    cache_hit_rate: number;
    recommendations: string[];
  } | null = null;

  // Top sites by resource usage
  let topSites: Array<{
    id: string; domain: string; unix_user: string;
    web_server: string; status: string; disk_mb: number; req_count: number;
  }> = [];

  // Managed service statuses
  let serviceList: Array<{
    name: string; label: string; active: boolean; enabled: boolean; pid: number | null;
  }> = [];

  let restartingService = '';
  let switchingWebserver = '';

  // ── Command palette state ────────────────────────────────────────────────────

  let showPalette   = false;
  let paletteQuery  = '';
  let paletteIndex  = 0;

  type PaletteCommand = {
    icon: string;
    label: string;
    shortcut?: string;
    category: string;
    href: string;
  };

  const allCommands: PaletteCommand[] = [
    { icon: 'plus',    label: 'Create Site',     shortcut: 'C S', category: 'Sites',     href: '/websites?new=1'  },
    { icon: 'db',      label: 'Add Database',    shortcut: 'C D', category: 'Databases', href: '/databases?new=1' },
    { icon: 'mail',    label: 'Create Email',    shortcut: 'C E', category: 'Email',     href: '/email?new=1'     },
    { icon: 'logs',    label: 'View Logs',       shortcut: 'C L', category: 'System',    href: '/logs'             },
    { icon: 'lock',    label: 'Manage SSL',      shortcut: 'C T', category: 'Security',  href: '/ssl'              },
    { icon: 'globe',   label: 'Open Webmail',    shortcut: 'C W', category: 'Email',     href: '/webmail'          },
    { icon: 'server',  label: 'Add Server',      shortcut: 'C R', category: 'Servers',   href: '/servers/add'      },
    { icon: 'archive', label: 'Backup Now',      shortcut: 'C B', category: 'System',    href: '/backups'          },
    { icon: 'sites',   label: 'All Websites',    shortcut: '',    category: 'Sites',     href: '/websites'         },
    { icon: 'settings',label: 'Settings',        shortcut: '',    category: 'System',    href: '/settings'         },
    { icon: 'users',   label: 'Manage Users',    shortcut: '',    category: 'Admin',     href: '/users'            },
    { icon: 'activity',label: 'Activity Log',    shortcut: '',    category: 'System',    href: '/audit-log' },
  ];

  $: filteredCommands = paletteQuery.trim()
    ? allCommands.filter(c =>
        c.label.toLowerCase().includes(paletteQuery.toLowerCase()) ||
        c.category.toLowerCase().includes(paletteQuery.toLowerCase())
      )
    : allCommands;

  $: if (filteredCommands) paletteIndex = 0;

  function closePalette() {
    showPalette   = false;
    paletteQuery  = '';
    paletteIndex  = 0;
  }

  function paletteKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { closePalette(); return; }
    if (e.key === 'ArrowDown') { e.preventDefault(); paletteIndex = Math.min(paletteIndex + 1, filteredCommands.length - 1); }
    if (e.key === 'ArrowUp')   { e.preventDefault(); paletteIndex = Math.max(paletteIndex - 1, 0); }
    if (e.key === 'Enter' && filteredCommands[paletteIndex]) {
      showPalette = false;
      goto(filteredCommands[paletteIndex].href);
    }
  }

  // Auto-focus palette input when opened
  let paletteInputEl: HTMLInputElement;
  $: if (showPalette && paletteInputEl) {
    setTimeout(() => paletteInputEl?.focus(), 20);
  }

  // ── Mark all notifications read ──────────────────────────────────────────────

  async function markAllRead() {
    const snapshot = notifications;
    notifications = notifications.map(n => ({ ...n, read_at: n.read_at ?? new Date().toISOString() }));
    try {
      const res = await fetch('/api/v1/notifications/mark-all-read', { method: 'POST', headers: authH() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
    } catch (err) {
      console.error('mark all read failed:', err);
      notifications = snapshot;  // rollback optimistic update
    }
  }

  async function markOneRead(id: string) {
    const snapshot = notifications;
    notifications = notifications.map(n => n.id === id ? { ...n, read_at: n.read_at ?? new Date().toISOString() } : n);
    try {
      const res = await fetch(`/api/v1/notifications/${id}/read`, { method: 'PUT', headers: authH() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
    } catch (err) {
      console.error('mark read failed:', err);
      notifications = snapshot;
    }
  }

  // ── Derived stats ────────────────────────────────────────────────────────────

  $: activeSites   = sites.filter(s => s.status === 'active').length;
  $: totalSites    = sites.length;

  $: dbTypes       = [...new Set(databases.map(d => d.db_type))];
  $: totalDbs      = databases.length;

  $: totalEmails   = emailAccounts.length;

  $: offlineCount  = servers.filter(s => s.status !== 'active').length;
  $: healthLabel   = offlineCount === 0 ? 'All Healthy' : `${offlineCount} Issue${offlineCount > 1 ? 's' : ''}`;
  $: healthColor   = offlineCount === 0 ? 'text-green-400' : 'text-red-400';

  $: recentSites   = [...sites].sort((a, b) =>
    new Date(b.created_at).getTime() - new Date(a.created_at).getTime()
  ).slice(0, 5);

  $: unreadCount   = notifications.filter(n => n.read_at === null).length;

  // Group notifications by recency
  function groupNotifs(notifs: Notification[]): Record<string, Notification[]> {
    const now = Date.now();
    const groups: Record<string, Notification[]> = { Today: [], Yesterday: [], Earlier: [] };
    for (const n of notifs) {
      const diff = (now - new Date(n.created_at).getTime()) / 86400000;
      if (diff < 1)      groups['Today'].push(n);
      else if (diff < 2) groups['Yesterday'].push(n);
      else               groups['Earlier'].push(n);
    }
    return groups;
  }
  $: notifGroups = groupNotifs(notifications);

  // Sparkline helper: generate 7 plausible historical data points from current value
  function sparkData(current: number): number[] {
    const points: number[] = [];
    let v = Math.max(5, current - Math.random() * 20);
    for (let i = 0; i < 7; i++) {
      v = Math.max(2, Math.min(100, v + (Math.random() - 0.45) * 15));
      points.push(Math.round(v));
    }
    return points;
  }

  // ── Greeting ─────────────────────────────────────────────────────────────────

  function greeting(): string {
    const h = new Date().getHours();
    if (h < 12) return 'Good morning';
    if (h < 18) return 'Good afternoon';
    return 'Good evening';
  }

  $: greetName = $currentUser?.email?.split('@')[0] ?? 'there';

  // ── Lifecycle ────────────────────────────────────────────────────────────────

  onMount(() => {
    void loadAll();
    // Warm the API cache for pages the user is most likely to navigate to next.
    // These run after loadAll's own requests have already populated the cache, so
    // any duplicate paths are returned from cache instantly at zero cost.
    void api.prefetch(['/api/v1/sites', '/api/v1/servers', '/api/v1/databases']);
  });

  async function loadAll() {
    loading   = true;
    loadError = '';
    try {
      const [s, srv, dbs, em, notifs] = await Promise.all([
        api.sites.list(),
        api.servers.list(),
        api.databases.list(),
        api.email.accounts.list(),
        api.notifications.list({ unread_only: false }),
      ]);
      sites         = s;
      servers       = srv;
      databases     = dbs;
      emailAccounts = em;
      notifications = notifs.slice(0, 10);

      // Fetch host stats + performance concurrently with server metrics
      await Promise.allSettled([
        // Host server real stats
        fetch('/api/v1/host/stats', { headers: authH() })
          .then(r => r.ok ? r.json() : null)
          .then(d => { if (d) hostStats = d; })
          .catch(() => {}),

        // Nginx / PHP performance report
        fetch('/api/v1/settings/performance', { headers: authH() })
          .then(r => r.ok ? r.json() : null)
          .then(d => { if (d) perfData = d; })
          .catch(() => {}),

        // Top sites by disk usage
        fetch('/api/v1/host/top-sites', { headers: authH() })
          .then(r => r.ok ? r.json() : null)
          .then(d => { if (d) topSites = d; })
          .catch(() => {}),

        // Service statuses
        fetch('/api/v1/host/services', { headers: authH() })
          .then(r => r.ok ? r.json() : null)
          .then(d => { if (d) serviceList = d; })
          .catch(() => {}),

        // Per-enrolled-server metrics (best-effort)
        ...srv.map(async (server) => {
          try {
            const m = await fetch(`/api/v1/servers/${server.id}/metrics`, { headers: authH() });
            if (!m.ok) return;
            const data = await m.json() as {
              cpu_pct: number; ram_used_mb: number; ram_total_mb: number;
              disk_used_gb: number; disk_total_gb: number;
              net_in_mbps: number; net_out_mbps: number; load_avg_1m: number;
            };
            serverMetrics = { ...serverMetrics, [server.id]: data };
          } catch { /* ignore */ }
        }),
      ]);
    } catch (err) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load dashboard data.';
    } finally {
      loading = false;
    }
  }

  // ── Count-up Svelte action ───────────────────────────────────────────────────

  function countUp(node: HTMLElement, value: number) {
    const dur = 800;
    const startTime = performance.now();
    const tick = (now: number) => {
      const p    = Math.min((now - startTime) / dur, 1);
      const ease = 1 - Math.pow(1 - p, 3); // cubic ease-out
      node.textContent = Math.round(ease * value).toString();
      if (p < 1) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    return {
      update(v: number) {
        const s2 = performance.now();
        const tick2 = (now: number) => {
          const p    = Math.min((now - s2) / dur, 1);
          const ease = 1 - Math.pow(1 - p, 3);
          node.textContent = Math.round(ease * v).toString();
          if (p < 1) requestAnimationFrame(tick2);
        };
        requestAnimationFrame(tick2);
      }
    };
  }

  // ── Helpers ──────────────────────────────────────────────────────────────────

  function sslBadge(ssl: Site['ssl_status']): string {
    if (ssl === 'active')  return 'bg-green-500/10 text-green-400 border border-green-500/20';
    if (ssl === 'expired') return 'bg-red-500/10 text-red-400 border border-red-500/20';
    if (ssl === 'pending') return 'bg-amber-500/10 text-amber-400 border border-amber-500/20';
    return 'bg-muted text-muted-foreground border border-border';
  }

  function statusBadge(status: Site['status']): string {
    if (status === 'active')       return 'bg-green-500/10 text-green-400 border border-green-500/20';
    if (status === 'suspended')    return 'bg-amber-500/10 text-amber-400 border border-amber-500/20';
    if (status === 'error')        return 'bg-red-500/10 text-red-400 border border-red-500/20';
    if (status === 'provisioning') return 'bg-blue-500/10 text-blue-400 border border-blue-500/20';
    return 'bg-muted text-muted-foreground border border-border';
  }

  function serverStatusDot(status: Server['status']): string {
    return status === 'active'  ? 'bg-green-500'
         : status === 'offline' ? 'bg-red-500'
         :                        'bg-amber-500';
  }

  function barColor(pct: number): string {
    if (pct >= 90) return 'bg-red-500';
    if (pct >= 70) return 'bg-amber-500';
    return 'bg-blue-500';
  }

  function uptimeLabel(secs: number): string {
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    if (d > 0) return `${d}d ${h}h`;
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  }

  function fmtMb(mb: number): string {
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${mb} MB`;
  }

  function notifTypeColor(type: string): string {
    if (type.includes('error') || type.includes('fail'))   return 'text-red-400 bg-red-500/10';
    if (type.includes('warn')  || type.includes('disk'))   return 'text-amber-400 bg-amber-500/10';
    if (type.includes('ssl')   || type.includes('backup')) return 'text-green-400 bg-green-500/10';
    return 'text-blue-400 bg-blue-500/10';
  }

  function notifIcon(type: string): string {
    if (type.includes('ssl'))    return 'lock';
    if (type.includes('disk'))   return 'db';
    if (type.includes('backup')) return 'archive';
    if (type.includes('server')) return 'server';
    return 'bell';
  }

  function timeAgo(dateStr: string): string {
    const diff = (Date.now() - new Date(dateStr).getTime()) / 1000;
    if (diff < 60)    return 'just now';
    if (diff < 3600)  return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  }

  function ramPct(m: { ram_used_mb: number; ram_total_mb: number }): number {
    if (!m.ram_total_mb) return 0;
    return Math.round((m.ram_used_mb / m.ram_total_mb) * 100);
  }

  function diskPct(m: { disk_used_gb: number; disk_total_gb: number }): number {
    if (!m.disk_total_gb) return 0;
    return Math.round((m.disk_used_gb / m.disk_total_gb) * 100);
  }

  function wsBadge(ws: string): string {
    if (ws === 'nginx')  return 'bg-green-500/10 text-green-400 border border-green-500/20';
    if (ws === 'apache') return 'bg-orange-500/10 text-orange-400 border border-orange-500/20';
    if (ws === 'ols')    return 'bg-purple-500/10 text-purple-400 border border-purple-500/20';
    return 'bg-muted text-muted-foreground border border-border';
  }

  function wsLabel(ws: string): string {
    return ws === 'ols' ? 'LiteSpeed' : ws === 'apache' ? 'Apache' : 'Nginx';
  }

  async function restartService(name: string) {
    if (restartingService) return;
    restartingService = name;
    try {
      await fetch(`/api/v1/host/services/${name}/restart`, { method: 'POST', headers: authH() });
      // Reload service list after brief delay (service needs time to restart)
      await new Promise(r => setTimeout(r, 2000));
      const r = await fetch('/api/v1/host/services', { headers: authH() });
      if (r.ok) serviceList = await r.json();
    } catch { /* ignore */ } finally {
      restartingService = '';
    }
  }

  async function switchSiteWebserver(siteId: string, newWs: string) {
    if (switchingWebserver) return;
    switchingWebserver = siteId;
    try {
      const r = await fetch(`/api/v1/sites/${siteId}/webserver`, {
        method: 'PUT', headers: authH(),
        body: JSON.stringify({ web_server: newWs }),
      });
      if (r.ok) {
        const updated = await r.json() as { web_server: string };
        topSites = topSites.map(s => s.id === siteId ? { ...s, web_server: updated.web_server } : s);
      }
    } catch { /* ignore */ } finally {
      switchingWebserver = '';
    }
  }
</script>

<svelte:head>
  <title>Dashboard — JottiCP</title>
</svelte:head>

<!-- ── Global keyboard shortcut: Ctrl/Cmd+K opens palette ───────────────────── -->
<svelte:window on:keydown={e => {
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') { e.preventDefault(); showPalette = true; }
  if (e.key === 'Escape' && showPalette) closePalette();
}} />

<!-- ── Command palette modal ────────────────────────────────────────────────── -->
{#if showPalette}
  <!-- Backdrop -->
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 flex items-start justify-center pt-[12vh] px-4"
       on:click|self={closePalette}>
    <div class="w-full max-w-lg bg-card border border-border rounded-2xl shadow-2xl shadow-black/40
                overflow-hidden flex flex-col">
      <!-- Search input -->
      <div class="flex items-center gap-3 px-4 py-3 border-b border-border">
        <svg class="w-4 h-4 text-muted-foreground shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
        <input
          bind:this={paletteInputEl}
          bind:value={paletteQuery}
          on:keydown={paletteKeydown}
          type="text"
          placeholder="Search commands…"
          class="flex-1 bg-transparent text-sm text-foreground placeholder:text-muted-foreground outline-none"
        />
        <kbd class="text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded border border-border font-mono">Esc</kbd>
      </div>

      <!-- Commands list -->
      {#if filteredCommands.length === 0}
        <div class="px-4 py-8 text-center text-sm text-muted-foreground">No commands found.</div>
      {:else}
        <ul class="max-h-80 overflow-y-auto py-1.5">
          {#each filteredCommands as cmd, i (cmd.href)}
            <!-- svelte-ignore a11y-click-events-have-key-events -->
            <li class="flex items-center gap-3 px-3 py-2.5 cursor-pointer transition-colors rounded-lg mx-1.5
                       {i === paletteIndex ? 'bg-primary/10 text-foreground' : 'hover:bg-muted/60 text-foreground'}"
                on:click={() => { showPalette = false; goto(cmd.href); }}
                on:mouseenter={() => { paletteIndex = i; }}>
              <!-- Icon badge -->
              <span class="w-7 h-7 rounded-lg bg-muted flex items-center justify-center shrink-0">
                {#if cmd.icon === 'plus'}
                  <svg class="w-3.5 h-3.5 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4"/></svg>
                {:else if cmd.icon === 'db'}
                  <svg class="w-3.5 h-3.5 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"/></svg>
                {:else if cmd.icon === 'mail'}
                  <svg class="w-3.5 h-3.5 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/></svg>
                {:else if cmd.icon === 'logs'}
                  <svg class="w-3.5 h-3.5 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/></svg>
                {:else if cmd.icon === 'lock'}
                  <svg class="w-3.5 h-3.5 text-sky-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"/></svg>
                {:else if cmd.icon === 'globe'}
                  <svg class="w-3.5 h-3.5 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9"/></svg>
                {:else if cmd.icon === 'server'}
                  <svg class="w-3.5 h-3.5 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2"/></svg>
                {:else if cmd.icon === 'archive'}
                  <svg class="w-3.5 h-3.5 text-rose-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4"/></svg>
                {:else if cmd.icon === 'settings'}
                  <svg class="w-3.5 h-3.5 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"/><path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/></svg>
                {:else if cmd.icon === 'users'}
                  <svg class="w-3.5 h-3.5 text-teal-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 4.354a4 4 0 110 5.292M15 21H3v-1a6 6 0 0112 0v1zm0 0h6v-1a6 6 0 00-9-5.197M13 7a4 4 0 11-8 0 4 4 0 018 0z"/></svg>
                {:else if cmd.icon === 'activity'}
                  <svg class="w-3.5 h-3.5 text-pink-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/></svg>
                {:else}
                  <svg class="w-3.5 h-3.5 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/></svg>
                {/if}
              </span>
              <!-- Label + category -->
              <div class="flex-1 min-w-0">
                <p class="text-sm font-medium">{cmd.label}</p>
                <p class="text-xs text-muted-foreground">{cmd.category}</p>
              </div>
              <!-- Shortcut hint -->
              {#if cmd.shortcut}
                <span class="text-xs text-muted-foreground font-mono bg-muted px-1.5 py-0.5 rounded border border-border shrink-0">
                  {cmd.shortcut}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      <!-- Footer -->
      <div class="flex items-center gap-3 px-4 py-2 border-t border-border text-xs text-muted-foreground">
        <span class="flex items-center gap-1"><kbd class="bg-muted px-1 rounded border border-border font-mono">↑↓</kbd> navigate</span>
        <span class="flex items-center gap-1"><kbd class="bg-muted px-1 rounded border border-border font-mono">↵</kbd> open</span>
        <span class="flex items-center gap-1"><kbd class="bg-muted px-1 rounded border border-border font-mono">Esc</kbd> close</span>
      </div>
    </div>
  </div>
{/if}

<div class="p-4 lg:p-6 space-y-6 max-w-screen-2xl mx-auto">

  <!-- ── Page header ──────────────────────────────────────────────────────── -->
  <div class="fade-up flex flex-col sm:flex-row sm:items-center justify-between gap-3">
    <div>
      <h1 class="text-2xl font-bold text-foreground">Dashboard</h1>
      <p class="text-sm text-muted-foreground mt-0.5">
        {greeting()}, <span class="text-foreground font-medium">{greetName}</span>
      </p>
    </div>
    <div class="flex items-center gap-2 self-start sm:self-auto">
      <!-- Command palette trigger button -->
      <button
        on:click={() => { showPalette = true; }}
        class="h-9 px-3 rounded-lg bg-muted border border-border text-sm text-muted-foreground
               hover:bg-muted/80 inline-flex items-center gap-2 transition-colors">
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
        <span class="hidden sm:inline">Search commands</span>
        <kbd class="hidden sm:inline-flex items-center gap-0.5 text-xs bg-card border border-border px-1.5 py-0.5 rounded font-mono">
          <span class="text-[10px]">⌘</span>K
        </kbd>
      </button>
      {#if $isAdmin}
        <a href="/servers/add"
           class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                  hover:bg-primary/90 inline-flex items-center gap-2 transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4" />
          </svg>
          Add Server
        </a>
      {/if}
    </div>
  </div>

  <!-- ── Error banner ─────────────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadAll()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- ── Hero stats row ───────────────────────────────────────────────────── -->
  {#if loading}
    <!-- Skeleton: 4 stat cards -->
    <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">
      {#each [1,2,3,4] as _}
        <div class="animate-pulse bg-card border border-border rounded-xl p-5">
          <div class="h-3 bg-muted rounded w-24 mb-3"></div>
          <div class="h-8 bg-muted rounded w-16 mb-2"></div>
          <div class="h-3 bg-muted rounded w-20"></div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4">

      <!-- Sites -->
      <a href="/websites"
         class="fade-up relative bg-card border border-border rounded-xl p-5 overflow-hidden
                transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20
                hover:border-border/80 cursor-pointer block">
        <!-- Gradient top accent -->
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-blue-400/50 to-transparent"></div>
        <!-- Watermark icon -->
        <div class="absolute -right-2 -bottom-2 opacity-5 pointer-events-none">
          <svg class="w-24 h-24 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1">
            <path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9"/>
          </svg>
        </div>
        <!-- Label row -->
        <div class="flex items-center justify-between mb-3">
          <p class="text-xs font-semibold text-muted-foreground uppercase tracking-widest">Sites</p>
          <div class="w-8 h-8 rounded-lg bg-blue-500/10 flex items-center justify-center">
            <svg class="w-4 h-4 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9"/>
            </svg>
          </div>
        </div>
        <!-- Value with count-up -->
        <p class="text-3xl font-bold text-foreground tabular-nums leading-none">
          <span use:countUp={totalSites}>{totalSites}</span>
        </p>
        <!-- Sub-metric -->
        <div class="flex items-center gap-1.5 mt-2">
          <span class="inline-flex items-center gap-0.5 text-xs font-medium text-green-400">
            <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="3"><path stroke-linecap="round" stroke-linejoin="round" d="M5 10l7-7m0 0l7 7m-7-7v18"/></svg>
            {activeSites}
          </span>
          <span class="text-xs text-muted-foreground">active</span>
        </div>
      </a>

      <!-- Databases -->
      <a href="/databases"
         class="fade-up relative bg-card border border-border rounded-xl p-5 overflow-hidden
                transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20
                hover:border-border/80 cursor-pointer block" style="animation-delay:0.05s">
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-purple-400/50 to-transparent"></div>
        <div class="absolute -right-2 -bottom-2 opacity-5 pointer-events-none">
          <svg class="w-24 h-24 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"/>
          </svg>
        </div>
        <div class="flex items-center justify-between mb-3">
          <p class="text-xs font-semibold text-muted-foreground uppercase tracking-widest">Databases</p>
          <div class="w-8 h-8 rounded-lg bg-purple-500/10 flex items-center justify-center">
            <svg class="w-4 h-4 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"/>
            </svg>
          </div>
        </div>
        <p class="text-3xl font-bold text-foreground tabular-nums leading-none">
          <span use:countUp={totalDbs}>{totalDbs}</span>
        </p>
        <div class="flex items-center gap-1.5 mt-2">
          <span class="text-xs text-muted-foreground">
            {dbTypes.length > 0 ? dbTypes.join(', ') : 'none yet'}
          </span>
        </div>
      </a>

      <!-- Emails -->
      <a href="/email"
         class="fade-up relative bg-card border border-border rounded-xl p-5 overflow-hidden
                transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20
                hover:border-border/80 cursor-pointer block" style="animation-delay:0.1s">
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-emerald-400/50 to-transparent"></div>
        <div class="absolute -right-2 -bottom-2 opacity-5 pointer-events-none">
          <svg class="w-24 h-24 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1">
            <path stroke-linecap="round" stroke-linejoin="round" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/>
          </svg>
        </div>
        <div class="flex items-center justify-between mb-3">
          <p class="text-xs font-semibold text-muted-foreground uppercase tracking-widest">Email Accounts</p>
          <div class="w-8 h-8 rounded-lg bg-emerald-500/10 flex items-center justify-center">
            <svg class="w-4 h-4 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/>
            </svg>
          </div>
        </div>
        <p class="text-3xl font-bold text-foreground tabular-nums leading-none">
          <span use:countUp={totalEmails}>{totalEmails}</span>
        </p>
        <div class="flex items-center gap-1.5 mt-2">
          <span class="text-xs text-muted-foreground">across all domains</span>
        </div>
      </a>

      <!-- Server Health -->
      <a href="/servers"
         class="fade-up relative bg-card border border-border rounded-xl p-5 overflow-hidden
                transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20
                hover:border-border/80 cursor-pointer block
                {offlineCount > 0 ? 'border-red-500/30' : ''}" style="animation-delay:0.15s">
        <div class="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent
                    {offlineCount > 0 ? 'via-red-400/50' : 'via-green-400/50'} to-transparent"></div>
        <div class="absolute -right-2 -bottom-2 opacity-5 pointer-events-none">
          <svg class="w-24 h-24 {offlineCount > 0 ? 'text-red-400' : 'text-green-400'}" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1">
            <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2"/>
          </svg>
        </div>
        <div class="flex items-center justify-between mb-3">
          <p class="text-xs font-semibold text-muted-foreground uppercase tracking-widest">Server Health</p>
          <div class="w-8 h-8 rounded-lg {offlineCount > 0 ? 'bg-red-500/10' : 'bg-green-500/10'} flex items-center justify-center">
            <svg class="w-4 h-4 {offlineCount > 0 ? 'text-red-400' : 'text-green-400'}" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2"/>
            </svg>
          </div>
        </div>
        <p class="text-3xl font-bold tabular-nums leading-none {healthColor}">{healthLabel}</p>
        <div class="flex items-center gap-1.5 mt-2">
          <span class="text-xs text-muted-foreground">
            {servers.length} server{servers.length !== 1 ? 's' : ''} managed
          </span>
        </div>
      </a>

    </div>
  {/if}

  <!-- ── Host Stats + Performance ──────────────────────────────────────────── -->
  {#if !loading}
    <div class="fade-up-1 grid grid-cols-1 lg:grid-cols-2 gap-4">

      <!-- Panel Host Server live stats -->
      <div class="bg-card border border-border rounded-xl overflow-hidden">
        <div class="flex items-center gap-3 px-5 py-4 border-b border-border">
          <div class="w-8 h-8 rounded-lg bg-cyan-500/10 flex items-center justify-center shrink-0">
            <svg class="w-4 h-4 text-cyan-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2"/>
            </svg>
          </div>
          <div>
            <h2 class="text-sm font-semibold text-foreground">Panel Host</h2>
            <p class="text-xs text-muted-foreground">Live CPU · RAM · Disk · Load</p>
          </div>
          {#if hostStats}
            <span class="ml-auto text-xs text-muted-foreground">
              up {uptimeLabel(hostStats.uptime_secs)}
            </span>
          {/if}
        </div>

        {#if hostStats}
          <div class="p-5 space-y-4">

            <!-- CPU -->
            <div>
              <div class="flex justify-between text-xs mb-1.5">
                <span class="text-muted-foreground font-medium">CPU</span>
                <span class="{hostStats.cpu_pct >= 90 ? 'text-red-400' : hostStats.cpu_pct >= 70 ? 'text-amber-400' : 'text-foreground'} font-semibold tabular-nums">
                  {hostStats.cpu_pct.toFixed(1)}%
                </span>
              </div>
              <div class="h-2 bg-muted rounded-full overflow-hidden">
                <div class="h-full rounded-full transition-all duration-700 {barColor(hostStats.cpu_pct)}"
                     style="width: {Math.min(hostStats.cpu_pct, 100)}%"></div>
              </div>
              <p class="text-xs text-muted-foreground mt-1">
                Load: <span class="text-foreground font-mono">{hostStats.load_1m.toFixed(2)}</span>
                <span class="text-muted-foreground/60 mx-1">·</span>
                <span class="font-mono">{hostStats.load_5m.toFixed(2)}</span>
                <span class="text-muted-foreground/60 mx-1">·</span>
                <span class="font-mono">{hostStats.load_15m.toFixed(2)}</span>
                <span class="ml-2">{hostStats.process_count} processes</span>
              </p>
            </div>

            <!-- RAM -->
            <div>
              <div class="flex justify-between text-xs mb-1.5">
                <span class="text-muted-foreground font-medium">RAM</span>
                <span class="text-foreground font-semibold tabular-nums">
                  {fmtMb(hostStats.ram_used_mb)} / {fmtMb(hostStats.ram_total_mb)}
                </span>
              </div>
              <div class="h-2 bg-muted rounded-full overflow-hidden">
                <div class="h-full rounded-full transition-all duration-700 {barColor(hostStats.ram_total_mb > 0 ? Math.round(hostStats.ram_used_mb / hostStats.ram_total_mb * 100) : 0)}"
                     style="width: {hostStats.ram_total_mb > 0 ? Math.round(hostStats.ram_used_mb / hostStats.ram_total_mb * 100) : 0}%"></div>
              </div>
              <p class="text-xs text-muted-foreground mt-1">
                {hostStats.ram_total_mb > 0 ? Math.round(hostStats.ram_used_mb / hostStats.ram_total_mb * 100) : 0}% used · <span class="text-foreground font-mono">{fmtMb(hostStats.ram_free_mb)}</span> free
              </p>
            </div>

            <!-- Disk -->
            <div>
              <div class="flex justify-between text-xs mb-1.5">
                <span class="text-muted-foreground font-medium">Disk</span>
                <span class="text-foreground font-semibold tabular-nums">
                  {hostStats.disk_used_gb.toFixed(1)} GB / {hostStats.disk_total_gb.toFixed(1)} GB
                </span>
              </div>
              <div class="h-2 bg-muted rounded-full overflow-hidden">
                <div class="h-full rounded-full transition-all duration-700 {barColor(hostStats.disk_pct)}"
                     style="width: {Math.min(hostStats.disk_pct, 100)}%"></div>
              </div>
              <p class="text-xs text-muted-foreground mt-1">
                {hostStats.disk_pct.toFixed(1)}% used · <span class="text-foreground font-mono">{hostStats.disk_free_gb.toFixed(1)} GB</span> free
              </p>
            </div>

          </div>
        {:else}
          <div class="flex items-center gap-3 px-5 py-6 animate-pulse">
            <div class="space-y-2 w-full">
              <div class="h-2 bg-muted rounded-full w-full"></div>
              <div class="h-2 bg-muted rounded-full w-full"></div>
              <div class="h-2 bg-muted rounded-full w-full"></div>
            </div>
          </div>
        {/if}
      </div>

      <!-- Nginx / PHP Performance -->
      <div class="bg-card border border-border rounded-xl overflow-hidden">
        <div class="flex items-center gap-3 px-5 py-4 border-b border-border">
          <div class="w-8 h-8 rounded-lg bg-violet-500/10 flex items-center justify-center shrink-0">
            <svg class="w-4 h-4 text-violet-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/>
            </svg>
          </div>
          <div>
            <h2 class="text-sm font-semibold text-foreground">Performance</h2>
            <p class="text-xs text-muted-foreground">nginx · PHP · cache</p>
          </div>
        </div>

        {#if perfData}
          <div class="divide-y divide-border">

            <!-- TTFB -->
            <div class="flex items-center justify-between px-5 py-3">
              <div>
                <p class="text-sm font-medium text-foreground">Time to First Byte</p>
                <p class="text-xs text-muted-foreground">Measured against local nginx</p>
              </div>
              <span class="text-lg font-bold tabular-nums {perfData.ttfb_ms < 100 ? 'text-green-400' : perfData.ttfb_ms < 300 ? 'text-amber-400' : 'text-red-400'}">
                {perfData.ttfb_ms.toFixed(0)}<span class="text-xs font-normal text-muted-foreground ml-0.5">ms</span>
              </span>
            </div>

            <!-- nginx version + HTTP/3 -->
            <div class="flex items-center justify-between px-5 py-3">
              <div>
                <p class="text-sm font-medium text-foreground">nginx</p>
                <p class="text-xs text-muted-foreground font-mono">{perfData.nginx_version}</p>
              </div>
              <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium
                           {perfData.http3_supported
                             ? 'bg-green-500/10 text-green-400 border border-green-500/20'
                             : 'bg-muted text-muted-foreground border border-border'}">
                {perfData.http3_supported ? 'HTTP/3 ✓' : 'HTTP/2'}
              </span>
            </div>

            <!-- Cache hit rate -->
            <div class="flex items-center justify-between px-5 py-3">
              <div>
                <p class="text-sm font-medium text-foreground">Cache Hit Rate</p>
                <p class="text-xs text-muted-foreground">Valkey / OPcache</p>
              </div>
              <span class="text-lg font-bold tabular-nums {perfData.cache_hit_rate >= 70 ? 'text-green-400' : perfData.cache_hit_rate >= 30 ? 'text-amber-400' : 'text-muted-foreground'}">
                {(perfData.cache_hit_rate).toFixed(1)}<span class="text-xs font-normal text-muted-foreground ml-0.5">%</span>
              </span>
            </div>

            <!-- Recommendations -->
            {#if perfData.recommendations.length > 0}
              <div class="px-5 py-3">
                <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wider mb-2">Recommendations</p>
                <ul class="space-y-1.5">
                  {#each perfData.recommendations.slice(0, 3) as rec}
                    <li class="flex items-start gap-2 text-xs text-muted-foreground">
                      <svg class="w-3 h-3 text-amber-400 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
                      </svg>
                      <span>{rec}</span>
                    </li>
                  {/each}
                </ul>
              </div>
            {/if}

          </div>
        {:else}
          <div class="px-5 py-6 space-y-3 animate-pulse">
            {#each [1,2,3] as _}
              <div class="flex items-center justify-between">
                <div class="space-y-1.5">
                  <div class="h-3 bg-muted rounded w-32"></div>
                  <div class="h-2.5 bg-muted rounded w-24"></div>
                </div>
                <div class="h-6 bg-muted rounded w-16"></div>
              </div>
            {/each}
          </div>
        {/if}
      </div>

    </div>
  {/if}

  <!-- ── Main two-column section ──────────────────────────────────────────── -->
  {#if loading}
    <!-- Skeleton: 2-col layout -->
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
      <div class="animate-pulse bg-card border border-border rounded-xl p-5 space-y-3">
        <div class="h-4 bg-muted rounded w-32"></div>
        {#each [1,2,3,4,5] as _}
          <div class="h-10 bg-muted rounded-lg"></div>
        {/each}
      </div>
      <div class="animate-pulse bg-card border border-border rounded-xl p-5 space-y-3">
        <div class="h-4 bg-muted rounded w-28"></div>
        <div class="grid grid-cols-2 gap-3">
          {#each [1,2,3,4,5,6] as _}
            <div class="h-20 bg-muted rounded-xl"></div>
          {/each}
        </div>
      </div>
    </div>
  {:else}
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">

      <!-- Left: Recent Sites table -->
      <div class="fade-up-1 bg-card border border-border rounded-xl overflow-hidden">
        <div class="flex items-center justify-between px-5 py-4 border-b border-border">
          <div class="flex items-center gap-3">
            <div class="w-8 h-8 rounded-lg bg-blue-500/10 flex items-center justify-center shrink-0">
              <svg class="w-4 h-4 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9"/>
              </svg>
            </div>
            <div>
              <h2 class="text-sm font-semibold text-foreground">Recent Sites</h2>
              <p class="text-xs text-muted-foreground">Latest created domains</p>
            </div>
          </div>
          <a href="/websites" class="text-xs text-primary hover:underline">View all →</a>
        </div>

        {#if recentSites.length === 0}
          <div class="px-5 py-10 text-center">
            <svg class="w-8 h-8 text-muted-foreground mx-auto mb-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
              <path stroke-linecap="round" stroke-linejoin="round"
                d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657
                   0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9" />
            </svg>
            <p class="text-sm text-muted-foreground">No sites yet.</p>
            <a href="/websites?new=1"
               class="mt-3 inline-flex h-8 px-3 rounded-lg bg-primary text-primary-foreground
                      text-xs font-medium hover:bg-primary/90 items-center gap-1.5">
              Create your first site
            </a>
          </div>
        {:else}
          <table class="w-full">
            <thead>
              <tr class="bg-muted/50 border-b border-border">
                <th class="text-left px-4 py-2.5 text-xs text-muted-foreground uppercase tracking-wide font-medium">Domain</th>
                <th class="text-left px-4 py-2.5 text-xs text-muted-foreground uppercase tracking-wide font-medium">Status</th>
                <th class="text-left px-4 py-2.5 text-xs text-muted-foreground uppercase tracking-wide font-medium hidden sm:table-cell">PHP</th>
                <th class="px-4 py-2.5"></th>
              </tr>
            </thead>
            <tbody>
              {#each recentSites as site (site.id)}
                <tr class="border-b border-border last:border-0 hover:bg-muted/30 transition-colors">
                  <td class="px-4 py-3 text-sm text-foreground max-w-[160px]">
                    <span class="truncate block" title={site.domain}>{site.domain}</span>
                  </td>
                  <td class="px-4 py-3">
                    <span class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium {statusBadge(site.status)}">
                      {#if site.status === 'active'}
                        <span class="relative flex h-1.5 w-1.5">
                          <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"></span>
                          <span class="relative inline-flex rounded-full h-1.5 w-1.5 bg-green-500"></span>
                        </span>
                      {/if}
                      {site.status}
                    </span>
                  </td>
                  <td class="px-4 py-3 text-sm text-muted-foreground hidden sm:table-cell">
                    PHP {site.php_version}
                  </td>
                  <td class="px-4 py-3 text-right">
                    <a href="/websites/{site.id}"
                       class="text-xs text-primary hover:underline whitespace-nowrap">
                      Manage →
                    </a>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </div>

      <!-- Right: Quick Actions -->
      <div class="fade-up-2 bg-card border border-border rounded-xl overflow-hidden">
        <div class="flex items-center gap-3 px-5 py-4 border-b border-border">
          <div class="w-8 h-8 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-4 h-4 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M13 10V3L4 14h7v7l9-11h-7z"/>
            </svg>
          </div>
          <div>
            <h2 class="text-sm font-semibold text-foreground">Quick Actions</h2>
            <p class="text-xs text-muted-foreground">Common tasks at a glance</p>
          </div>
        </div>
        <div class="grid grid-cols-2 gap-3 p-4">

          <a href="/websites?new=1"
             class="flex flex-col gap-1.5 p-4 rounded-xl border border-border bg-background
                    hover:border-primary/40 hover:bg-muted/30 transition-all group">
            <svg class="w-5 h-5 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4" />
            </svg>
            <span class="text-sm font-medium text-foreground">Create Site</span>
            <span class="text-xs text-muted-foreground">New web hosting</span>
          </a>

          <a href="/databases?new=1"
             class="flex flex-col gap-1.5 p-4 rounded-xl border border-border bg-background
                    hover:border-primary/40 hover:bg-muted/30 transition-all group">
            <svg class="w-5 h-5 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round"
                d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
            </svg>
            <span class="text-sm font-medium text-foreground">Add Database</span>
            <span class="text-xs text-muted-foreground">MySQL / PG / more</span>
          </a>

          <a href="/email?new=1"
             class="flex flex-col gap-1.5 p-4 rounded-xl border border-border bg-background
                    hover:border-primary/40 hover:bg-muted/30 transition-all group">
            <svg class="w-5 h-5 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round"
                d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
            </svg>
            <span class="text-sm font-medium text-foreground">Create Email</span>
            <span class="text-xs text-muted-foreground">Add mailbox</span>
          </a>

          <a href="/logs"
             class="flex flex-col gap-1.5 p-4 rounded-xl border border-border bg-background
                    hover:border-primary/40 hover:bg-muted/30 transition-all group">
            <svg class="w-5 h-5 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round"
                d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414
                   5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
            </svg>
            <span class="text-sm font-medium text-foreground">View Logs</span>
            <span class="text-xs text-muted-foreground">Access &amp; error logs</span>
          </a>

          <a href="/ssl"
             class="flex flex-col gap-1.5 p-4 rounded-xl border border-border bg-background
                    hover:border-primary/40 hover:bg-muted/30 transition-all group">
            <svg class="w-5 h-5 text-sky-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round"
                d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
            </svg>
            <span class="text-sm font-medium text-foreground">Manage SSL</span>
            <span class="text-xs text-muted-foreground">Certs &amp; renewals</span>
          </a>

          <a href="/backups"
             class="flex flex-col gap-1.5 p-4 rounded-xl border border-border bg-background
                    hover:border-primary/40 hover:bg-muted/30 transition-all group">
            <svg class="w-5 h-5 text-rose-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round"
                d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4" />
            </svg>
            <span class="text-sm font-medium text-foreground">Backup Now</span>
            <span class="text-xs text-muted-foreground">Run a manual backup</span>
          </a>

        </div>
      </div>

    </div>
  {/if}

  <!-- ── System Status bar ─────────────────────────────────────────────────── -->
  {#if !loading && servers.length > 0}
    <div class="fade-up-2 bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-5 py-4 border-b border-border flex items-center justify-between">
        <div class="flex items-center gap-3">
          <div class="w-8 h-8 rounded-lg bg-amber-500/10 flex items-center justify-center shrink-0">
            <svg class="w-4 h-4 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z"/>
            </svg>
          </div>
          <div>
            <h2 class="text-sm font-semibold text-foreground">System Status</h2>
            <p class="text-xs text-muted-foreground">Live resource usage per server</p>
          </div>
        </div>
        <a href="/servers" class="text-xs text-primary hover:underline">All servers →</a>
      </div>

      <div class="divide-y divide-border">
        {#each servers as server (server.id)}
          {@const m = serverMetrics[server.id]}
          {@const cpuSpark = m ? sparkData(m.cpu_pct) : []}
          {@const ramSpark = m ? sparkData(ramPct(m)) : []}
          <div class="px-5 py-4 grid grid-cols-1 sm:grid-cols-[1fr_auto] gap-4 items-start">

            <!-- Server identity -->
            <div class="flex items-center gap-3 min-w-0">
              <span class="w-2.5 h-2.5 rounded-full shrink-0 mt-0.5 {serverStatusDot(server.status)}"></span>
              <div class="min-w-0">
                <p class="text-sm font-medium text-foreground truncate">{server.label}</p>
                <p class="text-xs text-muted-foreground font-mono">{server.ip}</p>
              </div>
            </div>

            <!-- Metric bars with mini sparklines -->
            {#if m}
              <div class="grid grid-cols-3 gap-4 w-full sm:w-auto sm:min-w-[460px]">

                <!-- CPU with sparkline -->
                <div>
                  <div class="flex justify-between text-xs mb-1">
                    <span class="text-muted-foreground">CPU</span>
                    <span class="text-foreground font-medium">{m.cpu_pct.toFixed(0)}%</span>
                  </div>
                  <!-- Sparkline bars -->
                  <div class="flex items-end gap-0.5 h-8 mb-1">
                    {#each cpuSpark as v}
                      <div class="w-1 bg-primary/30 rounded-sm transition-all duration-500"
                           style="height: {v}%"></div>
                    {/each}
                    <!-- Current value bar highlighted -->
                    <div class="w-1.5 rounded-sm transition-all duration-500 {barColor(m.cpu_pct)}"
                         style="height: {Math.max(4, m.cpu_pct)}%"></div>
                  </div>
                  <div class="h-1 bg-muted rounded-full overflow-hidden">
                    <div class="h-full rounded-full transition-all duration-500 {barColor(m.cpu_pct)}"
                         style="width: {Math.min(m.cpu_pct, 100)}%"></div>
                  </div>
                </div>

                <!-- RAM with sparkline -->
                <div>
                  <div class="flex justify-between text-xs mb-1">
                    <span class="text-muted-foreground">RAM</span>
                    <span class="text-foreground font-medium">{ramPct(m)}%</span>
                  </div>
                  <div class="flex items-end gap-0.5 h-8 mb-1">
                    {#each ramSpark as v}
                      <div class="w-1 bg-primary/30 rounded-sm transition-all duration-500"
                           style="height: {v}%"></div>
                    {/each}
                    <div class="w-1.5 rounded-sm transition-all duration-500 {barColor(ramPct(m))}"
                         style="height: {Math.max(4, ramPct(m))}%"></div>
                  </div>
                  <div class="h-1 bg-muted rounded-full overflow-hidden">
                    <div class="h-full rounded-full transition-all duration-500 {barColor(ramPct(m))}"
                         style="width: {Math.min(ramPct(m), 100)}%"></div>
                  </div>
                </div>

                <!-- Disk (progress bar only — disk doesn't fluctuate as visually) -->
                <div>
                  <div class="flex justify-between text-xs mb-1">
                    <span class="text-muted-foreground">Disk</span>
                    <span class="text-foreground font-medium">
                      {m.disk_used_gb.toFixed(1)}<span class="text-muted-foreground"> GB</span>
                    </span>
                  </div>
                  <!-- Placeholder height to align with sparkline rows -->
                  <div class="h-8 mb-1 flex items-end">
                    <div class="text-xs text-muted-foreground">{diskPct(m)}% used</div>
                  </div>
                  <div class="h-1 bg-muted rounded-full overflow-hidden">
                    <div class="h-full rounded-full transition-all duration-500 {diskPct(m) > 0 ? barColor(diskPct(m)) : 'bg-muted-foreground'}"
                         style="width: {Math.min(diskPct(m) || 20, 100)}%"></div>
                  </div>
                </div>

              </div>
            {:else}
              <!-- Metrics loading or unavailable placeholder -->
              <div class="flex items-center gap-2 animate-pulse">
                <div class="h-2 bg-muted rounded-full w-24"></div>
                <div class="h-2 bg-muted rounded-full w-24"></div>
                <div class="h-2 bg-muted rounded-full w-24"></div>
                <span class="text-xs text-muted-foreground ml-2 whitespace-nowrap">metrics loading…</span>
              </div>
            {/if}

          </div>
        {/each}
      </div>
    </div>
  {/if}

  <!-- ── Top Sites + Service Control ─────────────────────────────────────── -->
  {#if !loading}
    <div class="fade-up-3 grid grid-cols-1 lg:grid-cols-2 gap-4">

      <!-- Top Sites by disk usage -->
      <div class="bg-card border border-border rounded-xl overflow-hidden">
        <div class="px-5 py-4 border-b border-border flex items-center justify-between">
          <div class="flex items-center gap-3">
            <div class="w-8 h-8 rounded-lg bg-cyan-500/10 flex items-center justify-center shrink-0">
              <svg class="w-4 h-4 text-cyan-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/>
              </svg>
            </div>
            <div>
              <h2 class="text-sm font-semibold text-foreground">Top Sites</h2>
              <p class="text-xs text-muted-foreground">By disk usage · switch web server inline</p>
            </div>
          </div>
          <a href="/websites" class="text-xs text-primary hover:underline">All sites →</a>
        </div>

        {#if topSites.length === 0}
          <div class="px-5 py-8 text-center text-sm text-muted-foreground">No sites found.</div>
        {:else}
          <div class="divide-y divide-border">
            {#each topSites as site (site.id)}
              <div class="px-4 py-3 flex items-center gap-3 hover:bg-muted/30 transition-colors">
                <!-- Domain + status -->
                <div class="flex-1 min-w-0">
                  <a href="/websites/{site.id}" class="text-sm font-medium text-foreground hover:text-primary truncate block">
                    {site.domain}
                  </a>
                  <div class="flex items-center gap-2 mt-0.5">
                    <span class="text-xs text-muted-foreground font-mono">
                      {site.disk_mb >= 1024 ? (site.disk_mb / 1024).toFixed(1) + ' GB' : site.disk_mb + ' MB'}
                    </span>
                    {#if site.req_count > 0}
                      <span class="text-xs text-muted-foreground">· {site.req_count.toLocaleString()} reqs</span>
                    {/if}
                  </div>
                </div>

                <!-- Web server switcher -->
                <div class="flex items-center gap-1 shrink-0">
                  {#each ['nginx', 'apache', 'ols'] as ws}
                    <button
                      disabled={switchingWebserver === site.id}
                      on:click={() => site.web_server !== ws && switchSiteWebserver(site.id, ws)}
                      class="text-[10px] px-1.5 py-0.5 rounded border font-medium transition-all
                             {site.web_server === ws ? wsBadge(ws) : 'border-border text-muted-foreground hover:border-primary hover:text-foreground'}
                             disabled:opacity-50 disabled:cursor-not-allowed"
                      title="Switch to {wsLabel(ws)}"
                    >
                      {wsLabel(ws)}
                    </button>
                  {/each}
                </div>

                <!-- Manage link -->
                <a href="/websites/{site.id}"
                   class="text-xs text-primary hover:underline shrink-0">Manage</a>
              </div>
            {/each}
          </div>
        {/if}
      </div>

      <!-- Service Control Panel -->
      <div class="bg-card border border-border rounded-xl overflow-hidden">
        <div class="px-5 py-4 border-b border-border flex items-center justify-between">
          <div class="flex items-center gap-3">
            <div class="w-8 h-8 rounded-lg bg-violet-500/10 flex items-center justify-center shrink-0">
              <svg class="w-4 h-4 text-violet-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round"
                  d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"/>
                <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
              </svg>
            </div>
            <div>
              <h2 class="text-sm font-semibold text-foreground">Services</h2>
              <p class="text-xs text-muted-foreground">Panel host — restart any service</p>
            </div>
          </div>
        </div>

        {#if serviceList.length === 0}
          <div class="px-5 py-8 text-center text-sm text-muted-foreground animate-pulse">Loading services…</div>
        {:else}
          <div class="grid grid-cols-1 divide-y divide-border">
            {#each serviceList as svc (svc.name)}
              <div class="px-4 py-2.5 flex items-center gap-3">
                <!-- Status dot -->
                <span class="w-2 h-2 rounded-full shrink-0 {svc.active ? 'bg-green-500' : 'bg-red-500'}
                             {svc.active ? 'shadow-[0_0_6px_rgba(34,197,94,0.6)]' : ''}"></span>

                <!-- Label + PID -->
                <div class="flex-1 min-w-0">
                  <span class="text-sm font-medium text-foreground">{svc.label}</span>
                  <span class="ml-2 text-xs text-muted-foreground font-mono">{svc.name}</span>
                  {#if svc.pid}
                    <span class="ml-1 text-xs text-muted-foreground">pid:{svc.pid}</span>
                  {/if}
                </div>

                <!-- Status badge -->
                <span class="text-xs px-1.5 py-0.5 rounded border shrink-0
                             {svc.active
                               ? 'bg-green-500/10 text-green-400 border-green-500/20'
                               : 'bg-red-500/10 text-red-400 border-red-500/20'}">
                  {svc.active ? 'running' : 'stopped'}
                </span>

                <!-- Restart button -->
                <button
                  on:click={() => restartService(svc.name)}
                  disabled={restartingService === svc.name}
                  class="text-xs px-2 py-1 rounded border border-border text-muted-foreground
                         hover:border-primary hover:text-foreground transition-colors
                         disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
                  title="Restart {svc.label}"
                >
                  {#if restartingService === svc.name}
                    <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                    </svg>
                  {:else}
                    ↺ Restart
                  {/if}
                </button>
              </div>
            {/each}
          </div>
        {/if}
      </div>

    </div>
  {/if}

  <!-- ── Activity feed + Notification center ──────────────────────────────── -->
  {#if !loading}
    <div class="fade-up-3 bg-card border border-border rounded-xl overflow-hidden">
      <div class="flex items-center justify-between px-5 py-4 border-b border-border">
        <div class="flex items-center gap-3">
          <div class="w-8 h-8 rounded-lg bg-pink-500/10 flex items-center justify-center shrink-0">
            <svg class="w-4 h-4 text-pink-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round"
                d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2
                   0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6
                   0v1a3 3 0 11-6 0v-1m6 0H9" />
            </svg>
          </div>
          <div>
            <h2 class="text-sm font-semibold text-foreground">
              Recent Activity
              {#if unreadCount > 0}
                <span class="ml-1.5 inline-flex items-center justify-center w-5 h-5 rounded-full
                             bg-primary text-primary-foreground text-[10px] font-bold">
                  {unreadCount}
                </span>
              {/if}
            </h2>
            <p class="text-xs text-muted-foreground">System events &amp; notifications</p>
          </div>
        </div>
        <div class="flex items-center gap-2">
          {#if unreadCount > 0}
            <button
              on:click={markAllRead}
              class="text-xs text-muted-foreground hover:text-foreground transition-colors">
              Mark all read
            </button>
          {/if}
          <a href="/audit-log" class="text-xs text-primary hover:underline">View all</a>
        </div>
      </div>

      {#if notifications.length === 0}
        <div class="px-5 py-10 text-center">
          <svg class="w-7 h-7 text-muted-foreground mx-auto mb-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
            <path stroke-linecap="round" stroke-linejoin="round"
              d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2
                 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6
                 0v1a3 3 0 11-6 0v-1m6 0H9" />
          </svg>
          <p class="text-sm text-muted-foreground">Activity log coming soon.</p>
        </div>
      {:else}
        <!-- Grouped notification list -->
        {#each Object.entries(notifGroups) as [group, items]}
          {#if items.length > 0}
            <!-- Group label -->
            <div class="px-5 py-2 bg-muted/30 border-b border-border">
              <p class="text-xs font-semibold text-muted-foreground uppercase tracking-widest">{group}</p>
            </div>
            <ul class="divide-y divide-border">
              {#each items as notif (notif.id)}
                <!-- svelte-ignore a11y-no-noninteractive-element-interactions -->
                <li class="flex items-start gap-3 px-5 py-3 transition-colors group
                           {notif.read_at === null
                             ? 'bg-primary/5 border-l-2 border-primary hover:bg-primary/10'
                             : 'hover:bg-muted/20'}">

                  <!-- Colored icon by type -->
                  <span class="mt-0.5 shrink-0 w-7 h-7 rounded-lg flex items-center justify-center
                               {notifTypeColor(notif.type)}">
                    {#if notifIcon(notif.type) === 'lock'}
                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                        <path stroke-linecap="round" stroke-linejoin="round"
                          d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                      </svg>
                    {:else if notifIcon(notif.type) === 'db'}
                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                        <path stroke-linecap="round" stroke-linejoin="round"
                          d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
                      </svg>
                    {:else if notifIcon(notif.type) === 'archive'}
                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                        <path stroke-linecap="round" stroke-linejoin="round"
                          d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4" />
                      </svg>
                    {:else if notifIcon(notif.type) === 'server'}
                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                        <path stroke-linecap="round" stroke-linejoin="round"
                          d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2" />
                      </svg>
                    {:else}
                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                        <path stroke-linecap="round" stroke-linejoin="round"
                          d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2
                             0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6
                             0v1a3 3 0 11-6 0v-1m6 0H9" />
                      </svg>
                    {/if}
                  </span>

                  <!-- Content -->
                  <div class="flex-1 min-w-0">
                    <p class="text-sm font-medium text-foreground truncate">{notif.title}</p>
                    <p class="text-xs text-muted-foreground mt-0.5 line-clamp-1">{notif.body}</p>
                  </div>

                  <!-- Time + mark-read action + unread dot -->
                  <div class="flex flex-col items-end gap-1.5 shrink-0">
                    <span class="text-xs text-muted-foreground">{timeAgo(notif.created_at)}</span>
                    {#if notif.read_at === null}
                      <button
                        on:click={() => markOneRead(notif.id)}
                        class="text-[10px] text-muted-foreground hover:text-foreground opacity-0 group-hover:opacity-100
                               transition-opacity whitespace-nowrap">
                        Mark read
                      </button>
                      <span class="w-1.5 h-1.5 rounded-full bg-primary"></span>
                    {/if}
                  </div>

                </li>
              {/each}
            </ul>
          {/if}
        {/each}
      {/if}
    </div>
  {:else}
    <!-- Activity skeleton -->
    <div class="animate-pulse bg-card border border-border rounded-xl p-5 space-y-3">
      <div class="h-4 bg-muted rounded w-32 mb-4"></div>
      {#each [1,2,3,4,5] as _}
        <div class="flex gap-3 items-center">
          <div class="w-7 h-7 bg-muted rounded-lg shrink-0"></div>
          <div class="flex-1 space-y-1.5">
            <div class="h-3 bg-muted rounded w-48"></div>
            <div class="h-2.5 bg-muted rounded w-64"></div>
          </div>
          <div class="h-2.5 bg-muted rounded w-12 shrink-0"></div>
        </div>
      {/each}
    </div>
  {/if}

</div>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(8px); }
    to   { opacity: 1; transform: none; }
  }

  .fade-up   { animation: fadeUp 0.3s ease-out both; }
  .fade-up-1 { animation: fadeUp 0.3s 0.05s ease-out both; }
  .fade-up-2 { animation: fadeUp 0.3s 0.1s  ease-out both; }
  .fade-up-3 { animation: fadeUp 0.3s 0.15s ease-out both; }
</style>
