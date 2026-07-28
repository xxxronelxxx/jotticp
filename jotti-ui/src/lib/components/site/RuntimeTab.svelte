<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';
  import type { Site } from '$api/client';

  export let siteId: string;
  export let site: Site;

  // ── Auth ───────────────────────────────────────────────────────────────────
  function authH(): Record<string, string> {
    const t = get(auth).token;
    return t
      ? { 'Authorization': 'Bearer ' + t, 'Content-Type': 'application/json' }
      : { 'Content-Type': 'application/json' };
  }

  // ── Types ──────────────────────────────────────────────────────────────────
  interface RuntimeInfo {
    site_id:        string;
    runtime:        string;
    version:        string;
    runtime_status: string;
    runtime_error:  string | null;
    port:           number | null;
    entry_point:    string | null;
    message:        string;
  }

  interface NodeStatus {
    name:      string;
    status:    string;
    pid:       number | null;
    uptime:    number | null;
    restarts:  number;
    cpu:       number;
    memory_mb: number;
  }

  interface Toast { id: number; message: string; type: 'success' | 'error' }

  // ── State ──────────────────────────────────────────────────────────────────
  let runtimeInfo: RuntimeInfo | null = null;
  let runtimeLoading = true;

  let nodeStatus: NodeStatus | null = null;
  let nodeLoading = false;

  let nodeLogs: { stdout: string; stderr: string } | null = null;
  let pythonLogs: { stdout: string; stderr: string } | null = null;
  let logsLoading = false;
  let showLogs = false;

  // Form state
  let selectedRuntime = 'php';
  let selectedVersion = '';
  let entryPoint      = 'main:app';
  let settingRuntime  = false;

  let nodeAction: string | null = null;
  let toasts: Toast[] = [];
  let toastCounter = 0;

  // Polling interval for provisioning status
  let pollInterval: ReturnType<typeof setInterval> | null = null;

  const RUNTIMES = [
    { id: 'php',    label: 'PHP',     icon: '🐘', description: 'Default — configure version in PHP tab' },
    { id: 'nodejs', label: 'Node.js', icon: '⬡',  description: 'systemd process manager, WebSocket support' },
    { id: 'python', label: 'Python',  icon: '🐍', description: 'WSGI/ASGI — Flask, Django, FastAPI, AI' },
  ];

  const DEFAULT_VERSIONS: Record<string, string[]> = {
    nodejs: ['20', '18', '16'],
    python: ['3.12', '3.11', '3.10'],
    php:    [],
  };

  const ENTRY_POINT_HINTS: Record<string, string[]> = {
    python: [
      'main:app',
      'app:app',
      'wsgi:application',
      'myapp.wsgi:application',
      'asgi:application',
    ],
  };

  // ── Helpers ────────────────────────────────────────────────────────────────
  function showToast(message: string, type: 'success' | 'error' = 'success') {
    const id = ++toastCounter;
    toasts = [...toasts, { id, message, type }];
    setTimeout(() => { toasts = toasts.filter(t => t.id !== id); }, 5000);
  }

  function formatUptime(ms: number | null): string {
    if (!ms) return '—';
    const s = Math.floor(ms / 1000);
    if (s < 60)  return `${s}s`;
    const m = Math.floor(s / 60);
    if (m < 60)  return `${m}m`;
    const h = Math.floor(m / 60);
    if (h < 24)  return `${h}h ${m % 60}m`;
    return `${Math.floor(h / 24)}d ${h % 24}h`;
  }

  function nodeStatusColor(s: string): string {
    if (s === 'online')  return 'text-green-400 bg-green-500/10 border-green-500/20';
    if (s === 'stopped') return 'text-amber-400 bg-amber-500/10 border-amber-500/20';
    if (s === 'errored') return 'text-red-400 bg-red-500/10 border-red-500/20';
    return 'text-muted-foreground bg-muted border-border';
  }

  function runtimeStatusColor(s: string): string {
    if (s === 'active')       return 'text-green-400 bg-green-500/10 border-green-500/20';
    if (s === 'provisioning') return 'text-blue-400 bg-blue-500/10 border-blue-500/20';
    if (s === 'error')        return 'text-red-400 bg-red-500/10 border-red-500/20';
    return 'text-muted-foreground bg-muted border-border';
  }

  function isProvisioning(): boolean {
    return runtimeInfo?.runtime_status === 'provisioning';
  }

  // ── Polling ────────────────────────────────────────────────────────────────
  function startPoll() {
    if (pollInterval) return;
    pollInterval = setInterval(async () => {
      await loadRuntime(false);
      if (!isProvisioning()) stopPoll();
    }, 3000);
  }

  function stopPoll() {
    if (pollInterval) { clearInterval(pollInterval); pollInterval = null; }
  }

  // ── API ────────────────────────────────────────────────────────────────────
  async function loadRuntime(showLoader = true) {
    if (showLoader) runtimeLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/runtime`, { headers: authH() });
      if (res.ok) {
        const data = await res.json() as RuntimeInfo;
        runtimeInfo      = data;
        selectedRuntime  = data.runtime || 'php';
        selectedVersion  = data.version || '';
        entryPoint       = data.entry_point || 'main:app';
        if (data.runtime_status === 'active' && data.runtime === 'nodejs') {
          await loadNodeStatus();
        }
      }
    } catch { /* ignore */ } finally {
      if (showLoader) runtimeLoading = false;
    }
  }

  async function applyRuntime() {
    settingRuntime = true;
    try {
      const body: Record<string, string> = { runtime: selectedRuntime };
      if (selectedVersion) body.version = selectedVersion;
      if (selectedRuntime === 'python') body.entry_point = entryPoint;

      const res = await fetch(`/api/v1/sites/${siteId}/runtime`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify(body),
      });
      const data = await res.json() as RuntimeInfo;
      if (!res.ok) {
        const msg = (data as unknown as { message?: string }).message ?? `HTTP ${res.status}`;
        throw new Error(msg);
      }
      runtimeInfo = data;
      if (data.runtime_status === 'provisioning') {
        showToast('Provisioning started…', 'success');
        startPoll();
      } else {
        showToast(`Runtime set to ${selectedRuntime}`, 'success');
      }
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Failed to set runtime', 'error');
    } finally {
      settingRuntime = false;
    }
  }

  async function loadNodeStatus() {
    nodeLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/nodejs/status`, { headers: authH() });
      if (res.ok) nodeStatus = await res.json() as NodeStatus;
    } catch { /* ignore */ } finally {
      nodeLoading = false;
    }
  }

  async function nodeAction_(action: string) {
    nodeAction = action;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/nodejs/process`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ action }),
      });
      const data = await res.json() as { message?: string };
      if (!res.ok) throw new Error(data.message ?? `HTTP ${res.status}`);
      showToast(`systemd ${action} OK`, 'success');
      setTimeout(() => void loadNodeStatus(), 1000);
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : `systemd ${action} failed`, 'error');
    } finally {
      nodeAction = null;
    }
  }

  async function loadLogs() {
    logsLoading = true;
    showLogs = true;
    nodeLogs   = null;
    pythonLogs = null;
    try {
      if (runtimeInfo?.runtime === 'nodejs') {
        const res = await fetch(`/api/v1/sites/${siteId}/nodejs/logs`, { headers: authH() });
        if (res.ok) nodeLogs = await res.json() as { stdout: string; stderr: string };
      } else if (runtimeInfo?.runtime === 'python') {
        const res = await fetch(`/api/v1/sites/${siteId}/python/logs`, { headers: authH() });
        if (res.ok) pythonLogs = await res.json() as { stdout: string; stderr: string };
      }
    } catch { showToast('Failed to load logs', 'error'); }
    finally   { logsLoading = false; }
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await loadRuntime();
    if (isProvisioning()) startPoll();
  });

  onDestroy(() => stopPoll());

  $: if (selectedRuntime !== 'php' && !selectedVersion) {
    selectedVersion = DEFAULT_VERSIONS[selectedRuntime]?.[0] ?? '';
  }

  $: isChanged = selectedRuntime !== (runtimeInfo?.runtime ?? 'php')
              || (selectedRuntime !== 'php' && selectedVersion !== (runtimeInfo?.version ?? ''))
              || (selectedRuntime === 'python' && entryPoint !== (runtimeInfo?.entry_point ?? 'main:app'));
</script>

<div class="tab-content space-y-5">

  <!-- ── Runtime selector card ─────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-5">
    <div class="flex items-center justify-between mb-4">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4"/>
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">Language Runtime</h2>
      </div>
      {#if runtimeInfo && !runtimeLoading}
        <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border {runtimeStatusColor(runtimeInfo.runtime_status)}">
          {#if runtimeInfo.runtime_status === 'provisioning'}
            <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
            </svg>
          {:else}
            <span class="w-1.5 h-1.5 rounded-full bg-current {runtimeInfo.runtime_status === 'active' ? 'animate-pulse' : ''}"></span>
          {/if}
          {runtimeInfo.runtime === 'php' ? 'PHP' : runtimeInfo.runtime}
          {runtimeInfo.version ? ` ${runtimeInfo.version}` : ''}
          · {runtimeInfo.runtime_status}
        </span>
      {/if}
    </div>

    {#if runtimeLoading}
      <div class="space-y-3 animate-pulse">
        <div class="bg-muted rounded-xl h-20"></div>
        <div class="bg-muted rounded h-9 w-32"></div>
      </div>
    {:else}

      <!-- Runtime pills -->
      <div class="grid grid-cols-3 gap-2 mb-4">
        {#each RUNTIMES as rt}
          <button
            type="button"
            on:click={() => { selectedRuntime = rt.id; selectedVersion = ''; }}
            disabled={isProvisioning()}
            class="p-3 rounded-xl border text-left transition-all
                   {selectedRuntime === rt.id
                     ? 'border-primary bg-primary/10 ring-1 ring-primary/30'
                     : 'border-border hover:bg-muted/40 disabled:opacity-50'}"
          >
            <div class="flex items-center gap-1.5 mb-0.5">
              <span class="text-base leading-none">{rt.icon}</span>
              <span class="text-sm font-semibold {selectedRuntime === rt.id ? 'text-primary' : 'text-foreground'}">{rt.label}</span>
            </div>
            <p class="text-xs text-muted-foreground leading-snug">{rt.description}</p>
          </button>
        {/each}
      </div>

      <!-- Version selector -->
      {#if selectedRuntime !== 'php'}
        <div class="mb-4">
          <label class="block text-xs font-medium text-muted-foreground mb-2">Version</label>
          <div class="flex gap-2 flex-wrap">
            {#each DEFAULT_VERSIONS[selectedRuntime] ?? [] as ver}
              <button
                type="button"
                on:click={() => selectedVersion = ver}
                class="h-8 px-3 rounded-lg border text-sm transition-colors
                       {selectedVersion === ver
                         ? 'border-primary bg-primary/10 text-primary'
                         : 'border-border text-muted-foreground hover:bg-muted'}"
              >{ver}</button>
            {/each}
            <input
              type="text"
              placeholder="Custom…"
              value={(DEFAULT_VERSIONS[selectedRuntime] ?? []).includes(selectedVersion) ? '' : selectedVersion}
              on:input={(e) => selectedVersion = (e.target as HTMLInputElement).value}
              class="h-8 px-3 rounded-lg border border-border bg-background text-sm text-foreground
                     focus:outline-none focus:ring-2 focus:ring-ring w-24"
            />
          </div>
        </div>
      {/if}

      <!-- Python entry point -->
      {#if selectedRuntime === 'python'}
        <div class="mb-4">
          <label class="block text-xs font-medium text-muted-foreground mb-1.5">
            Entry point
            <span class="ml-1 font-normal opacity-70">— WSGI/ASGI module:callable</span>
          </label>
          <input
            type="text"
            bind:value={entryPoint}
            placeholder="main:app"
            class="h-9 w-full px-3 rounded-lg border border-border bg-background text-sm text-foreground
                   focus:outline-none focus:ring-2 focus:ring-ring mb-2"
          />
          <div class="flex flex-wrap gap-1.5">
            {#each ENTRY_POINT_HINTS.python as hint}
              <button
                type="button"
                on:click={() => entryPoint = hint}
                class="h-6 px-2 rounded text-[11px] font-mono border transition-colors
                       {entryPoint === hint
                         ? 'border-primary bg-primary/10 text-primary'
                         : 'border-border text-muted-foreground hover:bg-muted'}"
              >{hint}</button>
            {/each}
          </div>
        </div>
      {/if}

      <!-- Apply button -->
      <div class="flex items-center gap-3">
        <button
          disabled={settingRuntime || isProvisioning() || !isChanged}
          on:click={applyRuntime}
          class="h-9 px-5 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 transition-colors disabled:opacity-50 inline-flex items-center gap-2"
        >
          {#if settingRuntime || isProvisioning()}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
            </svg>
            {isProvisioning() ? 'Provisioning…' : 'Applying…'}
          {:else}
            Apply Runtime
          {/if}
        </button>

        {#if isProvisioning()}
          <span class="text-xs text-blue-400 animate-pulse">Setting up — this takes ~30s…</span>
        {/if}
      </div>

      <!-- Error banner -->
      {#if runtimeInfo?.runtime_status === 'error' && runtimeInfo.runtime_error}
        <div class="mt-4 bg-red-500/10 border border-red-500/20 rounded-xl p-3 text-xs text-red-300">
          <p class="font-semibold mb-1">Provisioning failed</p>
          <pre class="whitespace-pre-wrap font-mono opacity-90">{runtimeInfo.runtime_error}</pre>
        </div>
      {/if}

    {/if}
  </div>

  <!-- ── Active runtime info card ───────────────────────────────────────────── -->
  {#if runtimeInfo?.runtime_status === 'active' && runtimeInfo.runtime !== 'php'}
    <div class="bg-card border border-border rounded-xl p-5">
      <div class="flex items-center gap-2.5 mb-3">
        <div class="w-7 h-7 rounded-lg bg-green-500/10 flex items-center justify-center">
          <svg class="w-3.5 h-3.5 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">Runtime Active</h2>
      </div>

      <div class="grid grid-cols-2 sm:grid-cols-3 gap-3 text-xs mb-4">
        <div class="bg-muted/30 rounded-lg p-3">
          <p class="text-muted-foreground mb-0.5">Runtime</p>
          <p class="font-semibold text-foreground uppercase">{runtimeInfo.runtime} {runtimeInfo.version}</p>
        </div>
        <div class="bg-muted/30 rounded-lg p-3">
          <p class="text-muted-foreground mb-0.5">Port</p>
          <p class="font-mono font-semibold text-foreground">{runtimeInfo.port ?? '—'}</p>
        </div>
        {#if runtimeInfo.runtime === 'python'}
          <div class="bg-muted/30 rounded-lg p-3">
            <p class="text-muted-foreground mb-0.5">Entry point</p>
            <p class="font-mono font-semibold text-foreground">{runtimeInfo.entry_point ?? '—'}</p>
          </div>
        {/if}
      </div>

      <!-- Runtime-specific instructions -->
      {#if runtimeInfo.runtime === 'nodejs'}
        <div class="bg-blue-500/10 border border-blue-500/20 rounded-xl p-4 text-xs text-blue-200 space-y-1.5">
          <p class="font-semibold text-blue-300">How to deploy your Node.js app</p>
          <p>1. Upload your app files to <code class="bg-blue-500/20 px-1 rounded font-mono">/home/[unix_user]/public_html/</code></p>
          <p>2. Your app must listen on <code class="bg-blue-500/20 px-1 rounded font-mono">process.env.PORT</code> (= <strong>{runtimeInfo.port}</strong>)</p>
          <p>3. Entry point: <code class="bg-blue-500/20 px-1 rounded font-mono">server.js</code> — rename your main file to this</p>
          <p>4. Managed by systemd: <code class="bg-blue-500/20 px-1 rounded font-mono">jottiecp-nodejs-{site.domain}</code> — auto-restarts on crash</p>
        </div>
      {:else if runtimeInfo.runtime === 'python'}
        <div class="bg-green-500/10 border border-green-500/20 rounded-xl p-4 text-xs text-green-200 space-y-1.5">
          <p class="font-semibold text-green-300">How to deploy your Python app</p>
          <p>1. Upload your app files to <code class="bg-green-500/20 px-1 rounded font-mono">/home/[unix_user]/public_html/</code></p>
          <p>2. Install your deps: SSH in and run <code class="bg-green-500/20 px-1 rounded font-mono">/home/[unix_user]/venv/bin/pip install -r requirements.txt</code></p>
          <p>3. Your app must expose <code class="bg-green-500/20 px-1 rounded font-mono">{runtimeInfo.entry_point ?? 'main:app'}</code> on port <strong>{runtimeInfo.port}</strong></p>
          <p>4. Systemd unit <code class="bg-green-500/20 px-1 rounded font-mono">jottiecp-python-{site.domain}</code> auto-restarts your app</p>
        </div>
      {/if}
    </div>
  {/if}

  <!-- ── Node.js systemd manager ───────────────────────────────────────────── -->
  {#if runtimeInfo?.runtime === 'nodejs' && runtimeInfo.runtime_status === 'active'}
    <div class="bg-card border border-border rounded-xl p-5">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-amber-500/10 flex items-center justify-center">
            <span class="text-base leading-none">⬡</span>
          </div>
          <h2 class="text-sm font-semibold text-foreground">Node.js Process</h2>
        </div>
        <div class="flex items-center gap-2">
          <button disabled={nodeAction !== null} on:click={() => nodeAction_('restart')}
            class="h-7 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1 disabled:opacity-50">
            {#if nodeAction === 'restart'}<svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path></svg>{/if}
            Restart
          </button>
          <button on:click={loadLogs} disabled={logsLoading}
            class="h-7 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1">
            <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2"/>
            </svg>
            Logs
          </button>
        </div>
      </div>

      <div class="bg-muted/20 border border-border rounded-xl p-4 text-xs space-y-1 text-muted-foreground mb-4">
        <p>Managed by systemd: <code class="text-foreground font-mono">jottiecp-nodejs-{site.domain}</code></p>
        <p>Entry point: <code class="text-foreground font-mono">/home/{site.unix_user}/public_html/server.js</code></p>
        <p>Port: <code class="text-foreground font-mono">{runtimeInfo.port}</code></p>
      </div>

      {#if nodeStatus}
        <div class="flex items-center gap-3 mb-4">
          <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border {nodeStatusColor(nodeStatus.status)}">
            <span class="w-1.5 h-1.5 rounded-full bg-current {nodeStatus.status === 'online' ? 'animate-pulse' : ''}"></span>
            {nodeStatus.status}
            {#if nodeStatus.pid} · PID {nodeStatus.pid}{/if}
            {#if nodeStatus.restarts > 0} · {nodeStatus.restarts} restarts{/if}
          </span>
          <button on:click={() => void loadNodeStatus()} disabled={nodeLoading}
            class="h-6 px-2 rounded border border-border text-[11px] text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1">
            {#if nodeLoading}<svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path></svg>{/if}
            Refresh
          </button>
        </div>
      {/if}

      <!-- Logs panel -->
      {#if showLogs && (nodeLogs || logsLoading)}
        <div class="bg-[#0d1117] border border-border/40 rounded-xl overflow-hidden">
          <div class="flex items-center justify-between px-4 py-2 border-b border-border/40">
            <span class="text-xs font-medium text-muted-foreground">Node.js Logs (last 100 lines)</span>
            <button class="text-muted-foreground hover:text-foreground text-xs" on:click={() => { showLogs = false; nodeLogs = null; }}>Close</button>
          </div>
          {#if logsLoading}
            <p class="p-4 text-xs text-muted-foreground animate-pulse">Loading…</p>
          {:else if nodeLogs}
            {#if nodeLogs.stdout.trim()}
              <div class="px-4 pt-3 pb-1">
                <p class="text-[10px] uppercase tracking-wider text-muted-foreground/60 mb-1">stdout</p>
                <pre class="font-mono text-xs text-green-300 whitespace-pre-wrap max-h-48 overflow-y-auto">{nodeLogs.stdout}</pre>
              </div>
            {/if}
            {#if nodeLogs.stderr.trim()}
              <div class="px-4 pt-2 pb-3 border-t border-border/40">
                <p class="text-[10px] uppercase tracking-wider text-muted-foreground/60 mb-1">stderr</p>
                <pre class="font-mono text-xs text-red-300 whitespace-pre-wrap max-h-48 overflow-y-auto">{nodeLogs.stderr}</pre>
              </div>
            {/if}
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  <!-- ── Python systemd status ──────────────────────────────────────────────── -->
  {#if runtimeInfo?.runtime === 'python' && runtimeInfo.runtime_status === 'active'}
    <div class="bg-card border border-border rounded-xl p-5">
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-yellow-500/10 flex items-center justify-center">
            <span class="text-base leading-none">🐍</span>
          </div>
          <h2 class="text-sm font-semibold text-foreground">Python Process</h2>
        </div>
        <button on:click={loadLogs} disabled={logsLoading}
          class="h-7 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1">
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2"/>
          </svg>
          View Logs
        </button>
      </div>

      <div class="bg-muted/20 border border-border rounded-xl p-4 text-xs space-y-1 text-muted-foreground">
        <p>Managed by systemd: <code class="text-foreground font-mono">jottiecp-python-{site.domain}</code></p>
        <p>Venv: <code class="text-foreground font-mono">/home/{site.unix_user}/venv/</code></p>
        <p>Port: <code class="text-foreground font-mono">{runtimeInfo.port}</code> · Entry: <code class="text-foreground font-mono">{runtimeInfo.entry_point}</code></p>
      </div>

      <!-- Python logs panel -->
      {#if showLogs && (pythonLogs || logsLoading)}
        <div class="mt-4 bg-[#0d1117] border border-border/40 rounded-xl overflow-hidden">
          <div class="flex items-center justify-between px-4 py-2 border-b border-border/40">
            <span class="text-xs font-medium text-muted-foreground">App Logs (last 100 lines)</span>
            <button class="text-muted-foreground hover:text-foreground text-xs" on:click={() => { showLogs = false; pythonLogs = null; }}>Close</button>
          </div>
          {#if logsLoading}
            <p class="p-4 text-xs text-muted-foreground animate-pulse">Loading…</p>
          {:else if pythonLogs}
            {#if pythonLogs.stdout.trim()}
              <div class="px-4 pt-3 pb-1">
                <pre class="font-mono text-xs text-green-300 whitespace-pre-wrap max-h-48 overflow-y-auto">{pythonLogs.stdout}</pre>
              </div>
            {/if}
            {#if pythonLogs.stderr.trim()}
              <div class="px-4 pt-2 pb-3 border-t border-border/40">
                <pre class="font-mono text-xs text-red-300 whitespace-pre-wrap max-h-48 overflow-y-auto">{pythonLogs.stderr}</pre>
              </div>
            {/if}
          {/if}
        </div>
      {/if}
    </div>
  {/if}

</div>

<!-- ── Toasts ──────────────────────────────────────────────────────────────── -->
<div class="fixed bottom-5 right-5 z-[60] flex flex-col gap-2 pointer-events-none">
  {#each toasts as toast (toast.id)}
    <div class="pointer-events-auto flex items-center gap-3 px-4 py-3 rounded-xl shadow-lg border text-sm font-medium
      {toast.type === 'success' ? 'bg-card border-green-500/30 text-foreground' : 'bg-card border-red-500/30 text-foreground'}">
      {#if toast.type === 'success'}
        <svg class="w-4 h-4 text-green-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
        </svg>
      {:else}
        <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
        </svg>
      {/if}
      <span>{toast.message}</span>
    </div>
  {/each}
</div>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
</style>
