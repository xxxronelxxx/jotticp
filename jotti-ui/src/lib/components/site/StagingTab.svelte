<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import type { Site } from '$api/client';
  import { auth } from '$lib/stores/auth';

  // ── Props ──────────────────────────────────────────────────────────────────
  export let siteId: string;
  export let site: Site;

  // ── Types ──────────────────────────────────────────────────────────────────
  interface StagingInfo {
    id: string;
    staging_domain: string;
    staging_url: string;
    status: 'active' | 'provisioning' | 'error';
    created_at: string;
    last_synced_at: string | null;
  }

  interface DiffEntry {
    path: string;
    status: 'modified' | 'added' | 'deleted';
    size_diff_bytes: number;
  }

  interface Toast { id: number; message: string; type: 'success' | 'error' }

  // ── Auth helper ────────────────────────────────────────────────────────────
  function authH(): Record<string, string> {
    const t = get(auth).token;
    return { 'Content-Type': 'application/json', ...(t ? { Authorization: 'Bearer ' + t } : {}) };
  }

  // ── State ──────────────────────────────────────────────────────────────────
  type ViewState = 'loading' | 'none' | 'provisioning' | 'active';

  let viewState: ViewState = 'loading';
  let stagingInfo: StagingInfo | null = null;
  let diff: DiffEntry[] = [];
  let diffLoading = false;
  let showAllDiff = false;
  const MAX_DIFF = 100;

  let createLoading = false;
  let syncLoading = false;
  let pushLoading = false;
  let deleteLoading = false;

  let pushConfirmed = false;
  let showDeleteConfirm = false;

  let provisionSteps = [
    { label: 'Creating files', done: false },
    { label: 'Syncing database', done: false },
    { label: 'Setting up domain', done: false },
    { label: 'Done', done: false },
  ];
  let provisionStepIdx = 0;

  let toasts: Toast[] = [];
  let toastCounter = 0;
  let pollInterval: ReturnType<typeof setInterval> | null = null;
  let provisionTimer: ReturnType<typeof setInterval> | null = null;
  let showHelp = false;

  // ── Toast ──────────────────────────────────────────────────────────────────
  function showToast(message: string, type: 'success' | 'error' = 'success') {
    const id = ++toastCounter;
    toasts = [...toasts, { id, message, type }];
    setTimeout(() => { toasts = toasts.filter(t => t.id !== id); }, 4000);
  }

  // ── Helpers ────────────────────────────────────────────────────────────────
  function relTime(iso: string | null): string {
    if (!iso) return 'Never';
    const diff = Date.now() - new Date(iso).getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return 'Just now';
    if (mins < 60) return `${mins}m ago`;
    const hrs = Math.floor(mins / 60);
    if (hrs < 24) return `${hrs}h ago`;
    return `${Math.floor(hrs / 24)}d ago`;
  }

  function formatBytes(b: number): string {
    const abs = Math.abs(b);
    const sign = b >= 0 ? '+' : '-';
    if (abs < 1024) return sign + abs + ' B';
    if (abs < 1048576) return sign + (abs / 1024).toFixed(1) + ' KB';
    return sign + (abs / 1048576).toFixed(1) + ' MB';
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' });
  }

  // ── API calls ──────────────────────────────────────────────────────────────
  async function loadStaging() {
    viewState = 'loading';
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/staging`, { headers: authH() });
      if (res.status === 404) {
        viewState = 'none';
      } else if (res.ok) {
        stagingInfo = await res.json();
        viewState = stagingInfo?.status === 'provisioning' ? 'provisioning' : 'active';
        if (viewState === 'provisioning') startProvisionPoll();
        if (viewState === 'active') await loadDiff();
      } else {
        viewState = 'none';
      }
    } catch {
      viewState = 'none';
    }
  }

  async function createStaging() {
    createLoading = true;
    viewState = 'provisioning';
    resetProvisionSteps();
    startProvisionAnimation();
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/staging/create`, { method: 'POST', headers: authH() });
      if (res.ok) {
        stagingInfo = await res.json();
        startProvisionPoll();
      } else {
        showToast('Failed to create staging environment', 'error');
        viewState = 'none';
        stopProvisionAnimation();
      }
    } catch {
      showToast('Network error', 'error');
      viewState = 'none';
      stopProvisionAnimation();
    } finally {
      createLoading = false;
    }
  }

  async function syncStaging() {
    syncLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/staging/sync`, { method: 'POST', headers: authH() });
      if (res.ok) {
        showToast('Sync started…');
        startJobPoll(async () => {
          if (stagingInfo) stagingInfo = { ...stagingInfo, last_synced_at: new Date().toISOString() };
          await loadDiff();
          showToast('Sync complete');
        });
      } else {
        showToast('Failed to start sync', 'error');
        syncLoading = false;
      }
    } catch {
      showToast('Network error', 'error');
      syncLoading = false;
    }
  }

  async function pushToLive() {
    pushLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/staging/push-to-live`, { method: 'POST', headers: authH() });
      if (res.ok) {
        showToast('Push to live started…');
        startJobPoll(() => {
          showToast('Successfully pushed to production');
          pushConfirmed = false;
        });
      } else {
        showToast('Failed to push to live', 'error');
        pushLoading = false;
      }
    } catch {
      showToast('Network error', 'error');
      pushLoading = false;
    }
  }

  async function deleteStaging() {
    deleteLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/staging`, { method: 'DELETE', headers: authH() });
      if (res.ok) {
        stagingInfo = null;
        diff = [];
        viewState = 'none';
        showDeleteConfirm = false;
        showToast('Staging environment deleted');
      } else {
        showToast('Failed to delete staging', 'error');
      }
    } catch {
      showToast('Network error', 'error');
    } finally {
      deleteLoading = false;
    }
  }

  async function loadDiff() {
    diffLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/staging/diff`, { headers: authH() });
      if (res.ok) {
        const data: unknown = await res.json();
        diff = Array.isArray(data) ? data as DiffEntry[] : [];
      }
    } catch {
      diff = [];
    } finally {
      diffLoading = false;
    }
  }

  // ── Provision polling ──────────────────────────────────────────────────────
  function resetProvisionSteps() {
    provisionSteps = provisionSteps.map(s => ({ ...s, done: false }));
    provisionStepIdx = 0;
  }

  function startProvisionAnimation() {
    provisionTimer = setInterval(() => {
      if (provisionStepIdx < provisionSteps.length) {
        provisionSteps[provisionStepIdx] = { ...provisionSteps[provisionStepIdx], done: true };
        provisionSteps = [...provisionSteps];
        provisionStepIdx += 1;
      }
    }, 1500);
  }

  function stopProvisionAnimation() {
    if (provisionTimer) { clearInterval(provisionTimer); provisionTimer = null; }
  }

  function startProvisionPoll() {
    stopPolling();
    pollInterval = setInterval(async () => {
      try {
        const res = await fetch(`/api/v1/sites/${siteId}/staging`, { headers: authH() });
        if (res.ok) {
          const data: StagingInfo = await res.json();
          if (data.status === 'active') {
            stagingInfo = data;
            viewState = 'active';
            stopPolling();
            stopProvisionAnimation();
            provisionSteps = provisionSteps.map(s => ({ ...s, done: true }));
            await loadDiff();
            showToast('Staging environment is ready');
          } else if (data.status === 'error') {
            viewState = 'none';
            stopPolling();
            stopProvisionAnimation();
            showToast('Staging provisioning failed', 'error');
          }
        }
      } catch { /* ignore */ }
    }, 4000);
  }

  function startJobPoll(onComplete: () => void) {
    let attempts = 0;
    const interval = setInterval(async () => {
      attempts += 1;
      if (attempts > 30) {
        clearInterval(interval);
        syncLoading = false;
        pushLoading = false;
        return;
      }
      try {
        const res = await fetch(`/api/v1/sites/${siteId}/staging`, { headers: authH() });
        if (res.ok) {
          const data: StagingInfo = await res.json();
          if (data.status === 'active') {
            clearInterval(interval);
            syncLoading = false;
            pushLoading = false;
            onComplete();
          }
        }
      } catch { /* ignore */ }
    }, 3000);
  }

  function stopPolling() {
    if (pollInterval) { clearInterval(pollInterval); pollInterval = null; }
  }

  // ── Keyboard close ─────────────────────────────────────────────────────────
  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { showDeleteConfirm = false; }
  }

  // ── Derived ────────────────────────────────────────────────────────────────
  $: diffModified = diff?.filter(d => d.status === 'modified') ?? [];
  $: diffAdded    = diff?.filter(d => d.status === 'added') ?? [];
  $: diffDeleted  = diff?.filter(d => d.status === 'deleted') ?? [];
  $: diffVisible  = showAllDiff ? diff : diff.slice(0, MAX_DIFF);

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await loadStaging();
    window.addEventListener('keydown', onKeydown);
  });

  onDestroy(() => {
    stopPolling();
    stopProvisionAnimation();
    window.removeEventListener('keydown', onKeydown);
  });
</script>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }

  /* Help tooltip */
  .help-btn { display:inline-flex; align-items:center; justify-content:center; width:18px; height:18px; border-radius:50%; background:rgba(99,102,241,.15); border:1px solid rgba(99,102,241,.3); color:rgba(99,102,241,.9); font-size:10px; font-weight:700; cursor:pointer; flex-shrink:0; transition:background .15s; line-height:1; }
  .help-btn:hover { background:rgba(99,102,241,.3); }
  .help-popover { position:absolute; z-index:50; top:calc(100% + 8px); left:0; width:320px; background:#1e293b; border:1px solid #334155; border-radius:12px; padding:14px 16px; box-shadow:0 8px 32px rgba(0,0,0,.4); }
  .help-popover h4 { margin:0 0 8px; font-size:13px; font-weight:600; color:#e2e8f0; display:flex; align-items:center; gap:6px; }
  .help-popover p { margin:0 0 8px; font-size:12px; color:#94a3b8; line-height:1.55; }
  .help-popover .example { background:rgba(99,102,241,.1); border:1px solid rgba(99,102,241,.2); border-radius:8px; padding:8px 10px; font-size:12px; color:#a5b4fc; line-height:1.5; }
  .help-popover .example strong { color:#c7d2fe; }
  .help-anchor { position:relative; display:inline-flex; }
</style>

<!-- ── Main container ─────────────────────────────────────────────────────── -->
<!-- svelte-ignore a11y-no-static-element-interactions -->
<div class="tab-content space-y-5" on:click={() => showHelp = false}>

  <!-- ── Loading ─────────────────────────────────────────────────────────── -->
  {#if viewState === 'loading'}
    <div class="bg-card border border-border rounded-xl p-6 space-y-3 animate-pulse">
      <div class="h-5 bg-muted rounded w-1/3"></div>
      <div class="h-4 bg-muted rounded w-2/3"></div>
      <div class="h-9 bg-muted rounded w-48 mt-2"></div>
    </div>

  <!-- ── State A: No staging ──────────────────────────────────────────────── -->
  {:else if viewState === 'none'}
    <div class="bg-card border border-border rounded-xl p-10 flex flex-col items-center text-center gap-4">
      <div class="w-14 h-14 rounded-full bg-muted flex items-center justify-center text-2xl select-none">⚗️</div>
      <div>
        <div class="flex items-center justify-center gap-2">
          <h3 class="text-base font-semibold text-foreground">No Staging Environment</h3>
          <!-- svelte-ignore a11y-no-static-element-interactions -->
          <div class="help-anchor">
            <button class="help-btn" on:click|stopPropagation={() => showHelp = !showHelp} aria-label="What is staging?">?</button>
            {#if showHelp}
              <!-- svelte-ignore a11y-no-static-element-interactions -->
              <div class="help-popover" on:click|stopPropagation>
                <h4>⚗️ What is Staging?</h4>
                <p>Staging is a <strong style="color:#e2e8f0">private copy of your live website</strong> where you can test changes safely — without affecting real visitors.</p>
                <div class="example">
                  <strong>Real-life example:</strong><br>
                  You run a shop at <em>mystore.com</em>. You want to redesign the homepage. Instead of editing the live site and risking broken pages in front of customers, you click <em>"Create Staging"</em>. JottiCP makes an identical copy at <em>staging.mystore.com</em>. You redesign there, test everything, then click <em>"Push to Live"</em> — and your visitors only ever see the finished result.
                </div>
                <p style="margin-top:8px;margin-bottom:0">Think of it like a <strong style="color:#e2e8f0">rehearsal stage</strong> — the real show never sees the rehearsal.</p>
              </div>
            {/if}
          </div>
        </div>
        <p class="text-sm text-muted-foreground mt-1 max-w-md">
          Create a staging copy of your site to test changes safely before going live. Available on Pro plans.
        </p>
      </div>
      <button
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-60"
        disabled={createLoading}
        on:click={createStaging}
      >
        {#if createLoading}
          <span class="flex items-center gap-2">
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
            Creating…
          </span>
        {:else}
          Create Staging Environment
        {/if}
      </button>
    </div>

  <!-- ── State B: Provisioning ────────────────────────────────────────────── -->
  {:else if viewState === 'provisioning'}
    <div class="bg-card border border-border rounded-xl p-8 space-y-6">
      <div class="flex items-center gap-3">
        <svg class="w-5 h-5 animate-spin text-primary shrink-0" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
        <span class="text-sm font-medium text-foreground">Creating staging environment…</span>
      </div>
      <ol class="space-y-3">
        {#each provisionSteps as step, i}
          <li class="flex items-center gap-3">
            {#if step.done}
              <span class="w-6 h-6 rounded-full bg-green-500/15 border border-green-500/30 flex items-center justify-center shrink-0">
                <svg class="w-3.5 h-3.5 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>
              </span>
            {:else if i === provisionStepIdx}
              <span class="w-6 h-6 rounded-full bg-blue-500/15 border border-blue-500/30 flex items-center justify-center shrink-0">
                <svg class="w-3 h-3 animate-spin text-blue-400" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
              </span>
            {:else}
              <span class="w-6 h-6 rounded-full bg-muted border border-border flex items-center justify-center shrink-0">
                <span class="w-1.5 h-1.5 rounded-full bg-muted-foreground/40"></span>
              </span>
            {/if}
            <span class="text-sm {step.done ? 'text-foreground' : i === provisionStepIdx ? 'text-blue-400' : 'text-muted-foreground'}">{step.label}</span>
          </li>
        {/each}
      </ol>
    </div>

  <!-- ── State C: Active ──────────────────────────────────────────────────── -->
  {:else if viewState === 'active' && stagingInfo}

    <!-- Staging info card -->
    <div class="bg-card border border-border rounded-xl p-5 space-y-4">
      <div class="flex items-start justify-between gap-3 flex-wrap">
        <div class="space-y-1">
          <div class="flex items-center gap-2">
            <h3 class="text-sm font-semibold text-foreground">Staging Environment</h3>
            <!-- svelte-ignore a11y-no-static-element-interactions -->
            <div class="help-anchor">
              <button class="help-btn" on:click|stopPropagation={() => showHelp = !showHelp} aria-label="What is staging?">?</button>
              {#if showHelp}
                <!-- svelte-ignore a11y-no-static-element-interactions -->
                <div class="help-popover" on:click|stopPropagation>
                  <h4>⚗️ What is Staging?</h4>
                  <p>A <strong style="color:#e2e8f0">private copy of your live site</strong> where you test changes safely before pushing them to real visitors.</p>
                  <div class="example">
                    <strong>Real-life example:</strong><br>
                    You want to update your homepage design. Instead of editing the live site, you work here on the staging copy. When you're happy, click <em>"Push to Live"</em> — visitors only see the finished result, never the work-in-progress.
                  </div>
                  <p style="margin-top:8px;margin-bottom:0"><strong style="color:#e2e8f0">Sync from Live</strong> pulls the latest live files into staging. <strong style="color:#e2e8f0">Push to Live</strong> sends your staged changes to production.</p>
                </div>
              {/if}
            </div>
            {#if stagingInfo.status === 'active'}
              <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">
                <span class="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse"></span>
                Active
              </span>
            {:else if stagingInfo.status === 'provisioning'}
              <span class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-blue-500/10 text-blue-400 border border-blue-500/20">Provisioning</span>
            {:else}
              <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-red-500/10 text-red-400 border border-red-500/20">Error</span>
            {/if}
          </div>
          <div class="text-xs text-muted-foreground">Created {formatDate(stagingInfo.created_at)} · Last synced: {relTime(stagingInfo.last_synced_at)}</div>
        </div>
        <div class="flex items-center gap-2">
          <a
            href={stagingInfo.staging_url}
            target="_blank"
            rel="noopener noreferrer"
            class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors flex items-center gap-1.5"
          >
            Visit Staging
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/></svg>
          </a>
          <button
            class="h-9 px-4 rounded-lg border border-red-500/40 text-sm font-medium text-red-400 hover:bg-red-500/10 transition-colors"
            on:click={() => showDeleteConfirm = true}
          >Delete</button>
        </div>
      </div>
      <div class="flex items-center gap-2">
        <code class="flex-1 bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 font-mono text-sm text-slate-300 truncate">{stagingInfo.staging_domain}</code>
        <a href={stagingInfo.staging_url} target="_blank" rel="noopener noreferrer" class="h-9 px-3 rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex items-center">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"/></svg>
        </a>
      </div>
    </div>

    <!-- Sync from Live -->
    <div class="bg-card border border-border rounded-xl p-5 space-y-3">
      <div class="flex items-start justify-between gap-3">
        <div>
          <h3 class="text-sm font-semibold text-foreground">Sync from Live</h3>
          <p class="text-sm text-muted-foreground mt-0.5">Copy files and database from production to staging. This overwrites staging content.</p>
        </div>
        <div class="text-xs text-muted-foreground whitespace-nowrap pt-0.5">Last sync: {relTime(stagingInfo.last_synced_at)}</div>
      </div>
      <button
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-60 flex items-center gap-2"
        disabled={syncLoading}
        on:click={syncStaging}
      >
        {#if syncLoading}
          <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
          Syncing…
        {:else}
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
          Sync Now
        {/if}
      </button>
    </div>

    <!-- File Diff viewer -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-5 py-4 border-b border-border flex items-center justify-between">
        <div class="flex items-center gap-3">
          <h3 class="text-sm font-semibold text-foreground">Changes Since Last Push</h3>
          {#if diff.length > 0}
            <div class="flex items-center gap-1.5 text-xs text-muted-foreground">
              {#if diffModified.length > 0}<span class="px-1.5 py-0.5 rounded bg-yellow-500/10 text-yellow-400 border border-yellow-500/20">~{diffModified.length}</span>{/if}
              {#if diffAdded.length > 0}<span class="px-1.5 py-0.5 rounded bg-green-500/10 text-green-400 border border-green-500/20">+{diffAdded.length}</span>{/if}
              {#if diffDeleted.length > 0}<span class="px-1.5 py-0.5 rounded bg-red-500/10 text-red-400 border border-red-500/20">-{diffDeleted.length}</span>{/if}
            </div>
          {/if}
        </div>
        <button
          class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors flex items-center gap-1.5"
          disabled={diffLoading}
          on:click={loadDiff}
        >
          {#if diffLoading}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
          {:else}
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/></svg>
          {/if}
          Refresh Diff
        </button>
      </div>

      {#if diffLoading && diff.length === 0}
        <div class="p-5 space-y-2">
          {#each [1,2,3] as _}
            <div class="h-9 bg-muted rounded-lg animate-pulse"></div>
          {/each}
        </div>
      {:else if diff.length === 0}
        <div class="p-8 text-center text-sm text-muted-foreground">No changes since last sync</div>
      {:else}
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-border bg-muted/30">
                <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground w-24">Status</th>
                <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">File Path</th>
                <th class="px-4 py-2.5 text-right text-xs font-medium text-muted-foreground w-28">Size Change</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-border">
              {#each diffVisible as entry (entry.path)}
                <tr class="hover:bg-muted/20 transition-colors">
                  <td class="px-4 py-2.5">
                    {#if entry.status === 'modified'}
                      <span class="inline-flex px-2 py-0.5 rounded text-xs font-medium bg-yellow-500/10 text-yellow-400 border border-yellow-500/20">modified</span>
                    {:else if entry.status === 'added'}
                      <span class="inline-flex px-2 py-0.5 rounded text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">added</span>
                    {:else}
                      <span class="inline-flex px-2 py-0.5 rounded text-xs font-medium bg-red-500/10 text-red-400 border border-red-500/20">deleted</span>
                    {/if}
                  </td>
                  <td class="px-4 py-2.5">
                    <code class="font-mono text-xs text-foreground break-all">{entry.path}</code>
                  </td>
                  <td class="px-4 py-2.5 text-right">
                    {#if entry.size_diff_bytes !== 0}
                      <span class="font-mono text-xs {entry.size_diff_bytes > 0 ? 'text-green-400' : 'text-red-400'}">{formatBytes(entry.size_diff_bytes)}</span>
                    {:else}
                      <span class="text-muted-foreground text-xs">—</span>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if diff.length > MAX_DIFF && !showAllDiff}
          <div class="px-5 py-3 border-t border-border">
            <button
              class="text-xs text-primary hover:text-primary/80 transition-colors"
              on:click={() => showAllDiff = true}
            >Show all {diff.length} files</button>
          </div>
        {/if}
      {/if}
    </div>

    <!-- Push to Live -->
    <div class="bg-card border-2 border-yellow-500/30 rounded-xl p-5 space-y-4">
      <div class="flex items-center gap-2">
        <svg class="w-4 h-4 text-yellow-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z"/></svg>
        <h3 class="text-sm font-semibold text-yellow-400">Push to Live</h3>
      </div>
      <p class="text-sm text-muted-foreground">
        This will overwrite your production site with staging content. All live changes since the last sync will be lost.
      </p>
      <label class="flex items-start gap-3 cursor-pointer">
        <input
          type="checkbox"
          bind:checked={pushConfirmed}
          class="mt-0.5 w-4 h-4 rounded border-border bg-background accent-primary cursor-pointer"
        />
        <span class="text-sm text-foreground">I understand this will overwrite production</span>
      </label>
      <button
        class="h-9 px-4 rounded-lg bg-red-500 text-white text-sm font-medium hover:bg-red-600 transition-colors disabled:opacity-40 flex items-center gap-2"
        disabled={!pushConfirmed || pushLoading}
        on:click={pushToLive}
      >
        {#if pushLoading}
          <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
          Pushing to live…
        {:else}
          Push to Live
        {/if}
      </button>
    </div>
  {/if}
</div>

<!-- ── Delete confirm modal ───────────────────────────────────────────────── -->
{#if showDeleteConfirm}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={() => showDeleteConfirm = false}
  >
    <div class="bg-card border border-border rounded-xl p-6 w-full max-w-sm space-y-4 shadow-2xl">
      <h2 class="text-base font-semibold text-foreground">Delete staging environment?</h2>
      <p class="text-sm text-muted-foreground">This cannot be undone. All staging files and databases will be permanently removed.</p>
      <div class="flex justify-end gap-3">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={() => showDeleteConfirm = false}
        >Cancel</button>
        <button
          class="h-9 px-4 rounded-lg bg-red-500 text-white text-sm font-medium hover:bg-red-600 transition-colors disabled:opacity-60"
          disabled={deleteLoading}
          on:click={deleteStaging}
        >
          {deleteLoading ? 'Deleting…' : 'Delete'}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toasts ─────────────────────────────────────────────────────────────── -->
<div class="fixed bottom-5 right-5 z-50 flex flex-col gap-2 pointer-events-none">
  {#each toasts as t (t.id)}
    <div
      class="pointer-events-auto flex items-center gap-2.5 px-4 py-3 rounded-lg shadow-lg text-sm font-medium border animate-in slide-in-from-bottom-2 duration-200
        {t.type === 'success' ? 'bg-green-500/10 border-green-500/20 text-green-400' : 'bg-red-500/10 border-red-500/20 text-red-400'}"
    >
      {#if t.type === 'success'}
        <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>
      {:else}
        <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path stroke-linecap="round" stroke-linejoin="round" d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126z"/></svg>
      {/if}
      {t.message}
    </div>
  {/each}
</div>
