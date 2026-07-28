<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';

  // ── Types ─────────────────────────────────────────────────────────────────────

  interface AuditEntry {
    id:            string;
    user_id:       string | null;
    user_email:    string | null;
    action:        string;
    resource_type: string | null;
    resource_id:   string | null;
    ip_address:    string | null;
    created_at:    string;
  }

  // ── State ─────────────────────────────────────────────────────────────────────

  let entries: AuditEntry[] = [];
  let loading = true;
  let totalCount = 0;

  // Filters
  let filterAction    = '';
  let filterTarget    = '';
  let filterDateFrom  = '';
  let filterDateTo    = '';
  const PAGE_SIZE = 50;
  let page = 0;

  // Export
  let exporting = false;

  // ── Auth helpers ──────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const token = get(auth).token;
    return token ? { Authorization: 'Bearer ' + token } : {};
  }

  // ── Lifecycle ─────────────────────────────────────────────────────────────────

  onMount(loadEntries);

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleString('en-US', {
      month: 'short', day: 'numeric', year: 'numeric',
      hour: '2-digit', minute: '2-digit', second: '2-digit',
    });
  }

  function timeAgo(iso: string): string {
    const diff = (Date.now() - new Date(iso).getTime()) / 1000;
    if (diff < 60)    return 'just now';
    if (diff < 3600)  return `${Math.floor(diff / 60)}m ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
    return `${Math.floor(diff / 86400)}d ago`;
  }

  function actionColor(action: string): string {
    if (action.includes('delete') || action.includes('suspend')) return 'text-red-400';
    if (action.includes('create') || action.includes('install') || action.includes('enable')) return 'text-green-400';
    if (action.includes('update') || action.includes('change') || action.includes('modify')) return 'text-blue-400';
    if (action.includes('login') || action.includes('auth')) return 'text-purple-400';
    return 'text-muted-foreground';
  }

  function actionIcon(action: string): string {
    if (action.includes('delete') || action.includes('remove') || action.includes('drop')) {
      return 'M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16';
    }
    if (action.includes('create') || action.includes('add') || action.includes('install')) {
      return 'M12 9v3m0 0v3m0-3h3m-3 0H9m12 0a9 9 0 11-18 0 9 9 0 0118 0z';
    }
    if (action.includes('login')) {
      return 'M11 16l-4-4m0 0l4-4m-4 4h14m-5 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h7a3 3 0 013 3v1';
    }
    if (action.includes('update') || action.includes('change') || action.includes('modify')) {
      return 'M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z';
    }
    return 'M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2';
  }

  function buildQuery(): string {
    const params = new URLSearchParams();
    if (filterAction)   params.set('action',    filterAction);
    if (filterTarget)   params.set('target',    filterTarget);
    if (filterDateFrom) params.set('date_from', new Date(filterDateFrom).toISOString());
    if (filterDateTo)   params.set('date_to',   new Date(filterDateTo + 'T23:59:59').toISOString());
    params.set('limit',  String(PAGE_SIZE));
    params.set('offset', String(page * PAGE_SIZE));
    return params.toString();
  }

  // ── Data loading ──────────────────────────────────────────────────────────────

  async function loadEntries() {
    loading = true;
    try {
      const res = await fetch(`/api/v1/audit-log?${buildQuery()}`, { headers: authH() });
      if (res.ok) {
        entries = await res.json() as AuditEntry[];
        // Server doesn't return total count — infer pagination from result size
        totalCount = page * PAGE_SIZE + entries.length;
      }
    } catch { /* ignore */ }
    finally { loading = false; }
  }

  function applyFilters() {
    page = 0;
    void loadEntries();
  }

  function clearFilters() {
    filterAction   = '';
    filterTarget   = '';
    filterDateFrom = '';
    filterDateTo   = '';
    page = 0;
    void loadEntries();
  }

  function nextPage() { page++; void loadEntries(); }
  function prevPage() { if (page > 0) { page--; void loadEntries(); } }

  // ── CSV Export ────────────────────────────────────────────────────────────────

  async function exportCsv() {
    exporting = true;
    try {
      const params = new URLSearchParams();
      if (filterAction)   params.set('action',    filterAction);
      if (filterTarget)   params.set('target',    filterTarget);
      if (filterDateFrom) params.set('date_from', new Date(filterDateFrom).toISOString());
      if (filterDateTo)   params.set('date_to',   new Date(filterDateTo + 'T23:59:59').toISOString());
      params.set('limit', '10000');

      const res = await fetch(`/api/v1/audit-log?${params}`, {
        headers: { ...authH(), Accept: 'text/csv' },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `audit-log-${new Date().toISOString().slice(0, 10)}.csv`;
      a.click();
      URL.revokeObjectURL(url);
    } catch { /* ignore */ }
    finally { exporting = false; }
  }

  // ── Common action strings for filter dropdown ─────────────────────────────────
  const COMMON_ACTIONS = [
    'site.created', 'site.deleted', 'site.suspended', 'site.unsuspended',
    'site.ssl_issued', 'site.php_changed', 'site.cloned',
    'user.login', 'user.logout', 'user.created', 'user.password_changed',
    'database.created', 'database.deleted',
    'email.account_created', 'email.account_deleted',
    'backup.started', 'backup.completed', 'backup.restored',
    'dns.record_created', 'dns.record_deleted',
    'server.enrolled', 'server.deleted',
  ];

  const RESOURCE_TYPES = ['site', 'user', 'database', 'email', 'backup', 'dns', 'server', 'ssl', 'cron', 'domain'];
</script>

<svelte:head>
  <title>Audit Log — JottiCP</title>
</svelte:head>

<div class="space-y-6 page-content">

  <!-- Header ───────────────────────────────────────────────────────────────── -->
  <div class="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
    <div>
      <h1 class="text-2xl font-bold text-foreground tracking-tight">Audit Log</h1>
      <p class="text-sm text-muted-foreground mt-0.5">Complete record of all admin actions and system events</p>
    </div>
    <button
      on:click={exportCsv}
      disabled={exporting}
      class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
             hover:bg-muted hover:text-foreground transition-colors inline-flex items-center gap-2 shrink-0
             disabled:opacity-50 disabled:cursor-not-allowed"
    >
      {#if exporting}
        <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
          <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
          <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
        </svg>
        Exporting…
      {:else}
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/>
        </svg>
        Export CSV
      {/if}
    </button>
  </div>

  <!-- Filters ──────────────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-4">
    <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-3">
      <!-- Action filter -->
      <div>
        <label class="block text-xs font-medium text-muted-foreground mb-1.5">Action</label>
        <select
          bind:value={filterAction}
          class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        >
          <option value="">All actions</option>
          {#each COMMON_ACTIONS as action}
            <option value={action}>{action}</option>
          {/each}
        </select>
      </div>

      <!-- Resource type filter -->
      <div>
        <label class="block text-xs font-medium text-muted-foreground mb-1.5">Resource type</label>
        <select
          bind:value={filterTarget}
          class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        >
          <option value="">All resources</option>
          {#each RESOURCE_TYPES as rt}
            <option value={rt}>{rt}</option>
          {/each}
        </select>
      </div>

      <!-- Date from -->
      <div>
        <label class="block text-xs font-medium text-muted-foreground mb-1.5">From date</label>
        <input
          type="date"
          bind:value={filterDateFrom}
          class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        />
      </div>

      <!-- Date to -->
      <div>
        <label class="block text-xs font-medium text-muted-foreground mb-1.5">To date</label>
        <input
          type="date"
          bind:value={filterDateTo}
          class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        />
      </div>
    </div>

    <div class="flex items-center gap-2 mt-3">
      <button
        on:click={applyFilters}
        class="h-8 px-4 rounded-lg bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90 transition-colors"
      >
        Apply Filters
      </button>
      {#if filterAction || filterTarget || filterDateFrom || filterDateTo}
        <button
          on:click={clearFilters}
          class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
        >
          Clear
        </button>
      {/if}
    </div>
  </div>

  <!-- Table ────────────────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden">
    <div class="px-5 py-4 border-b border-border flex items-center justify-between">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2"/>
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">
          {#if entries.length > 0}
            Showing {page * PAGE_SIZE + 1}–{page * PAGE_SIZE + entries.length} entries
          {:else}
            Audit entries
          {/if}
        </h2>
      </div>
      <button
        on:click={loadEntries}
        class="h-7 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1.5"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
        </svg>
        Refresh
      </button>
    </div>

    {#if loading}
      <div class="divide-y divide-border">
        {#each [0, 1, 2, 3, 4, 5, 6, 7] as _}
          <div class="flex items-center gap-4 px-5 py-3">
            <div class="animate-pulse bg-muted rounded w-7 h-7 shrink-0"></div>
            <div class="flex-1 space-y-1.5">
              <div class="animate-pulse bg-muted rounded h-3.5 w-48"></div>
              <div class="animate-pulse bg-muted rounded h-2.5 w-32"></div>
            </div>
            <div class="animate-pulse bg-muted rounded h-3 w-20 shrink-0"></div>
          </div>
        {/each}
      </div>
    {:else if entries.length === 0}
      <div class="px-5 py-16 text-center">
        <div class="w-12 h-12 mx-auto mb-3 rounded-full bg-muted flex items-center justify-center">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2"/>
          </svg>
        </div>
        <p class="text-sm font-medium text-foreground mb-1">No audit entries found</p>
        <p class="text-xs text-muted-foreground">
          {filterAction || filterTarget || filterDateFrom || filterDateTo
            ? 'Try changing or clearing your filters'
            : 'System actions will appear here as they occur'}
        </p>
      </div>
    {:else}
      <!-- Desktop table -->
      <div class="hidden md:block overflow-x-auto">
        <table class="w-full text-sm">
          <thead class="border-b border-border bg-muted/30">
            <tr>
              <th class="px-5 py-2.5 text-left text-xs font-medium text-muted-foreground w-8"></th>
              <th class="px-3 py-2.5 text-left text-xs font-medium text-muted-foreground">Action</th>
              <th class="px-3 py-2.5 text-left text-xs font-medium text-muted-foreground">Resource</th>
              <th class="px-3 py-2.5 text-left text-xs font-medium text-muted-foreground">User</th>
              <th class="px-3 py-2.5 text-left text-xs font-medium text-muted-foreground">IP</th>
              <th class="px-3 py-2.5 text-right text-xs font-medium text-muted-foreground pr-5">Time</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-border">
            {#each entries as entry (entry.id)}
              <tr class="hover:bg-muted/20 transition-colors">
                <!-- Icon -->
                <td class="px-5 py-3">
                  <div class="w-6 h-6 rounded-lg flex items-center justify-center">
                    <svg class="w-3.5 h-3.5 {actionColor(entry.action)}" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                      <path stroke-linecap="round" stroke-linejoin="round" d={actionIcon(entry.action)}/>
                    </svg>
                  </div>
                </td>
                <!-- Action -->
                <td class="px-3 py-3">
                  <span class="font-mono text-xs font-medium {actionColor(entry.action)}">{entry.action}</span>
                </td>
                <!-- Resource -->
                <td class="px-3 py-3">
                  {#if entry.resource_type}
                    <div class="flex flex-col">
                      <span class="text-xs font-medium text-foreground capitalize">{entry.resource_type}</span>
                      {#if entry.resource_id}
                        <span class="text-xs text-muted-foreground font-mono">{entry.resource_id.slice(0, 8)}…</span>
                      {/if}
                    </div>
                  {:else}
                    <span class="text-xs text-muted-foreground">—</span>
                  {/if}
                </td>
                <!-- User -->
                <td class="px-3 py-3">
                  <span class="text-xs text-foreground">{entry.user_email ?? 'System'}</span>
                </td>
                <!-- IP -->
                <td class="px-3 py-3">
                  <span class="text-xs font-mono text-muted-foreground">{entry.ip_address ?? '—'}</span>
                </td>
                <!-- Time -->
                <td class="px-3 py-3 pr-5 text-right">
                  <span class="text-xs text-muted-foreground" title={formatDate(entry.created_at)}>
                    {timeAgo(entry.created_at)}
                  </span>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

      <!-- Mobile list -->
      <div class="md:hidden divide-y divide-border">
        {#each entries as entry (entry.id)}
          <div class="px-4 py-3 flex items-start gap-3">
            <div class="w-7 h-7 rounded-lg bg-muted flex items-center justify-center shrink-0 mt-0.5">
              <svg class="w-3.5 h-3.5 {actionColor(entry.action)}" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d={actionIcon(entry.action)}/>
              </svg>
            </div>
            <div class="flex-1 min-w-0">
              <p class="font-mono text-xs font-medium {actionColor(entry.action)}">{entry.action}</p>
              <p class="text-xs text-muted-foreground mt-0.5">
                {entry.user_email ?? 'System'} · {entry.ip_address ?? '—'}
              </p>
              {#if entry.resource_type}
                <p class="text-xs text-muted-foreground">
                  {entry.resource_type}{entry.resource_id ? ` · ${entry.resource_id.slice(0,8)}…` : ''}
                </p>
              {/if}
            </div>
            <span class="text-xs text-muted-foreground shrink-0" title={formatDate(entry.created_at)}>
              {timeAgo(entry.created_at)}
            </span>
          </div>
        {/each}
      </div>
    {/if}

    <!-- Pagination -->
    {#if !loading && (entries.length === PAGE_SIZE || page > 0)}
      <div class="px-5 py-3 border-t border-border flex items-center justify-between">
        <button
          on:click={prevPage}
          disabled={page === 0}
          class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
        >
          ← Previous
        </button>
        <span class="text-xs text-muted-foreground">Page {page + 1}</span>
        <button
          on:click={nextPage}
          disabled={entries.length < PAGE_SIZE}
          class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
        >
          Next →
        </button>
      </div>
    {/if}
  </div>

</div>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(6px) } to { opacity:1; transform:none } }
  .page-content { animation: fadeUp 0.2s ease-out both }
</style>
