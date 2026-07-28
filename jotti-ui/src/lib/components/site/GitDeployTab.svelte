<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import type { Site } from '$api/client';
  import { auth } from '$lib/stores/auth';

  // ── Props ──────────────────────────────────────────────────────────────────
  export let siteId: string;
  export let site: Site;

  // ── Types ──────────────────────────────────────────────────────────────────
  interface GitStatus {
    enabled: boolean;
    remote_url: string | null;
    branch: string | null;
    deploy_key_pub: string | null;
    webhook_url: string | null;
  }

  interface DeployEntry {
    id: string;
    commit_hash: string;
    commit_message: string;
    deployed_at: string;
    deployed_by: string;
    status: 'success' | 'failed' | 'running';
    duration_ms: number | null;
    error?: string;
  }

  interface Toast { id: number; message: string; type: 'success' | 'error' }

  // ── Auth helper ────────────────────────────────────────────────────────────
  function authH(): Record<string, string> {
    const t = get(auth).token;
    return { 'Content-Type': 'application/json', ...(t ? { Authorization: 'Bearer ' + t } : {}) };
  }

  // ── State ──────────────────────────────────────────────────────────────────
  let gitStatus: GitStatus | null = null;
  let history: DeployEntry[] = [];
  let loading = true;
  let historyLoading = false;
  let historyPage = 1;
  const PAGE_SIZE = 20;

  let enableLoading = false;
  let disableLoading = false;
  let deployLoading = false;
  let saveLoading = false;

  let showDisableConfirm = false;
  let editingRemoteUrl = false;
  let editRemoteUrlValue = '';
  let editBranch = 'main';

  let activeProvider: 'github' | 'gitlab' | 'bitbucket' = 'github';
  let deployBranch = 'main';
  let expandedErrors: Record<string, boolean> = {};

  let toasts: Toast[] = [];
  let toastCounter = 0;
  let pollInterval: ReturnType<typeof setInterval> | null = null;

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

  function shortHash(h: string): string {
    return h.slice(0, 7);
  }

  async function copyText(text: string, label: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast(`${label} copied to clipboard`);
    } catch {
      showToast('Failed to copy', 'error');
    }
  }

  // ── API calls ──────────────────────────────────────────────────────────────
  async function loadStatus() {
    loading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/git-deploy/status`, { headers: authH() });
      if (res.ok) {
        gitStatus = await res.json();
        editBranch = gitStatus?.branch ?? 'main';
        deployBranch = gitStatus?.branch ?? 'main';
      } else {
        gitStatus = { enabled: false, remote_url: null, branch: null, deploy_key_pub: null, webhook_url: null };
      }
    } catch {
      gitStatus = { enabled: false, remote_url: null, branch: null, deploy_key_pub: null, webhook_url: null };
    } finally {
      loading = false;
    }
  }

  async function loadHistory(append = false) {
    historyLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/git-deploy/history?page=${historyPage}&per_page=${PAGE_SIZE}`, { headers: authH() });
      if (res.ok) {
        const data: DeployEntry[] = await res.json();
        history = append ? [...history, ...data] : data;
      }
    } finally {
      historyLoading = false;
    }
  }

  async function enableGitDeploy() {
    enableLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/git-deploy/enable`, { method: 'POST', headers: authH() });
      if (res.ok) {
        const data = await res.json();
        gitStatus = { enabled: true, remote_url: null, branch: 'main', deploy_key_pub: data.deploy_key_pub, webhook_url: data.webhook_url };
        editBranch = 'main';
        deployBranch = 'main';
        showToast('Git Deploy enabled successfully');
      } else {
        showToast('Failed to enable Git Deploy', 'error');
      }
    } catch {
      showToast('Network error', 'error');
    } finally {
      enableLoading = false;
    }
  }

  async function disableGitDeploy() {
    disableLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/git-deploy/disable`, { method: 'POST', headers: authH() });
      if (res.ok) {
        gitStatus = { enabled: false, remote_url: null, branch: null, deploy_key_pub: null, webhook_url: null };
        showDisableConfirm = false;
        history = [];
        showToast('Git Deploy disabled');
      } else {
        showToast('Failed to disable Git Deploy', 'error');
      }
    } catch {
      showToast('Network error', 'error');
    } finally {
      disableLoading = false;
    }
  }

  async function saveSettings() {
    if (!gitStatus) return;
    saveLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}`, {
        method: 'PUT',
        headers: authH(),
        body: JSON.stringify({ git_remote_url: editRemoteUrlValue || gitStatus.remote_url, git_branch: editBranch }),
      });
      if (res.ok) {
        gitStatus = { ...gitStatus, remote_url: editRemoteUrlValue || gitStatus.remote_url, branch: editBranch };
        deployBranch = editBranch;
        editingRemoteUrl = false;
        showToast('Settings saved');
      } else {
        showToast('Failed to save settings', 'error');
      }
    } catch {
      showToast('Network error', 'error');
    } finally {
      saveLoading = false;
    }
  }

  async function triggerDeploy() {
    deployLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/git-deploy/trigger`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ branch: deployBranch }),
      });
      if (res.ok) {
        showToast(`Deploying from ${deployBranch}...`);
        startPolling();
      } else {
        showToast('Failed to trigger deploy', 'error');
        deployLoading = false;
      }
    } catch {
      showToast('Network error', 'error');
      deployLoading = false;
    }
  }

  function startPolling() {
    stopPolling();
    pollInterval = setInterval(async () => {
      await loadHistory();
      const latest = history[0];
      if (latest && (latest.status === 'success' || latest.status === 'failed')) {
        stopPolling();
        deployLoading = false;
        if (latest.status === 'success') {
          showToast('Deployment successful');
        } else {
          showToast('Deployment failed', 'error');
        }
      }
    }, 3000);
  }

  function stopPolling() {
    if (pollInterval) { clearInterval(pollInterval); pollInterval = null; }
  }

  async function loadMore() {
    historyPage += 1;
    await loadHistory(true);
  }

  // ── Modal keyboard close ───────────────────────────────────────────────────
  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') showDisableConfirm = false;
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await loadStatus();
    if (gitStatus?.enabled) await loadHistory();
    window.addEventListener('keydown', onKeydown);
  });

  onDestroy(() => {
    stopPolling();
    window.removeEventListener('keydown', onKeydown);
  });

  // ── Provider instructions ──────────────────────────────────────────────────
  const providerInstructions: Record<string, { name: string; path: string }> = {
    github:    { name: 'GitHub',    path: 'Settings → Deploy keys → Add deploy key' },
    gitlab:    { name: 'GitLab',    path: 'Settings → Repository → Deploy keys' },
    bitbucket: { name: 'Bitbucket', path: 'Repository settings → Access keys → Add key' },
  };
</script>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
</style>

<!-- ── Main container ─────────────────────────────────────────────────────── -->
<div class="tab-content space-y-5">

  <!-- Loading skeleton -->
  {#if loading}
    <div class="bg-card border border-border rounded-xl p-6 space-y-3 animate-pulse">
      <div class="h-5 bg-muted rounded w-1/3"></div>
      <div class="h-4 bg-muted rounded w-2/3"></div>
      <div class="h-9 bg-muted rounded w-40 mt-2"></div>
    </div>

  {:else if gitStatus && !gitStatus.enabled}
    <!-- ── State A: Disabled ──────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-10 flex flex-col items-center text-center gap-4">
      <div class="w-14 h-14 rounded-full bg-muted flex items-center justify-center">
        <svg class="w-7 h-7 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M2.25 12c0-5.385 4.365-9.75 9.75-9.75s9.75 4.365 9.75 9.75-4.365 9.75-9.75 9.75S2.25 17.385 2.25 12zm14.03-4.72a.75.75 0 00-1.06 0l-4.72 4.72-2.22-2.22a.75.75 0 00-1.06 1.06l2.75 2.75a.75.75 0 001.06 0l5.25-5.25a.75.75 0 000-1.06z" />
        </svg>
      </div>
      <div>
        <h3 class="text-base font-semibold text-foreground">Git Deploy not enabled</h3>
        <p class="text-sm text-muted-foreground mt-1 max-w-md">
          Connect a Git repository to automatically deploy when you push code. Supports GitHub, GitLab, and Bitbucket.
        </p>
      </div>
      <button
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-60"
        disabled={enableLoading}
        on:click={enableGitDeploy}
      >
        {#if enableLoading}
          <span class="flex items-center gap-2">
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
            Enabling…
          </span>
        {:else}
          Enable Git Deploy
        {/if}
      </button>
    </div>

  {:else if gitStatus && gitStatus.enabled}
    <!-- ── State B: Enabled ───────────────────────────────────────────────── -->

    <!-- Hero status card -->
    <div class="bg-card border border-border rounded-xl p-5 space-y-4">
      <div class="flex items-center justify-between flex-wrap gap-3">
        <div class="flex items-center gap-3">
          <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">
            <span class="w-1.5 h-1.5 rounded-full bg-green-400 animate-pulse"></span>
            Git Deploy Active
          </span>
        </div>
        <button
          class="h-9 px-4 rounded-lg border border-red-500/40 text-sm font-medium text-red-400 hover:bg-red-500/10 transition-colors"
          on:click={() => showDisableConfirm = true}
        >
          Disable Git Deploy
        </button>
      </div>

      <!-- Remote URL -->
      <div class="space-y-1.5">
        <label class="text-xs font-medium text-muted-foreground uppercase tracking-wide">Remote URL</label>
        {#if editingRemoteUrl}
          <div class="flex gap-2">
            <input
              bind:value={editRemoteUrlValue}
              placeholder="https://github.com/user/repo.git"
              class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
            />
          </div>
        {:else}
          <div class="flex items-center gap-2">
            <span class="flex-1 bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 font-mono text-sm text-slate-300 truncate">
              {gitStatus.remote_url ?? 'Not set'}
            </span>
            <button
              class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted transition-colors"
              on:click={() => { editingRemoteUrl = true; editRemoteUrlValue = gitStatus?.remote_url ?? ''; }}
            >Edit</button>
          </div>
        {/if}
      </div>

      <!-- Branch -->
      <div class="space-y-1.5">
        <label class="text-xs font-medium text-muted-foreground uppercase tracking-wide">Branch</label>
        <input
          bind:value={editBranch}
          placeholder="main"
          class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        />
      </div>

      <button
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-60"
        disabled={saveLoading}
        on:click={saveSettings}
      >
        {saveLoading ? 'Saving…' : 'Save'}
      </button>
    </div>

    <!-- Deploy Key section -->
    <div class="bg-card border border-border rounded-xl p-5 space-y-4">
      <div class="flex items-center justify-between">
        <h3 class="text-sm font-semibold text-foreground">Deploy Key</h3>
        <!-- Provider tabs -->
        <div class="flex gap-1 bg-muted rounded-lg p-1">
          {#each ['github', 'gitlab', 'bitbucket'] as prov}
            <button
              class="px-3 py-1 rounded-md text-xs font-medium transition-colors {activeProvider === prov ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
              on:click={() => activeProvider = prov as typeof activeProvider}
            >{providerInstructions[prov].name}</button>
          {/each}
        </div>
      </div>

      {#if gitStatus.deploy_key_pub}
        <div class="relative">
          <pre class="bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 font-mono text-xs text-slate-300 whitespace-pre-wrap break-all leading-relaxed">{gitStatus.deploy_key_pub}</pre>
        </div>
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={() => copyText(gitStatus!.deploy_key_pub!, 'Deploy key')}
        >
          <span class="flex items-center gap-1.5">
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/></svg>
            Copy Key
          </span>
        </button>
      {:else}
        <div class="h-16 bg-muted rounded-lg animate-pulse"></div>
      {/if}

      <ol class="space-y-2 text-sm text-muted-foreground list-decimal list-inside">
        <li>Copy the key above</li>
        <li>Go to your repository → <span class="text-foreground font-medium">{providerInstructions[activeProvider].path}</span></li>
        <li>Paste the key. Leave "Allow write access" <strong class="text-foreground">unchecked</strong></li>
        <li>Push a commit to trigger your first deployment</li>
      </ol>
    </div>

    <!-- Webhook URL section -->
    <div class="bg-card border border-border rounded-xl p-5 space-y-4">
      <h3 class="text-sm font-semibold text-foreground">Webhook URL</h3>
      {#if gitStatus.webhook_url}
        <div class="flex items-center gap-2">
          <code class="flex-1 bg-slate-950 border border-slate-800 rounded-lg px-3 py-2 font-mono text-sm text-slate-300 truncate">{gitStatus.webhook_url}</code>
          <button
            class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted transition-colors whitespace-nowrap"
            on:click={() => copyText(gitStatus!.webhook_url!, 'Webhook URL')}
          >Copy URL</button>
        </div>
      {:else}
        <div class="h-10 bg-muted rounded-lg animate-pulse"></div>
      {/if}
      <p class="text-sm text-muted-foreground">Add this webhook URL in your repository settings to trigger deployments on push events.</p>
    </div>

    <!-- Manual Deploy -->
    <div class="bg-card border border-border rounded-xl p-5 space-y-4">
      <h3 class="text-sm font-semibold text-foreground">Manual Deploy</h3>
      <div class="flex items-center gap-3 flex-wrap">
        <div class="flex items-center gap-2">
          <label class="text-sm text-muted-foreground">Branch:</label>
          <input
            bind:value={deployBranch}
            class="h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring w-36"
          />
        </div>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-60 flex items-center gap-2"
          disabled={deployLoading}
          on:click={triggerDeploy}
        >
          {#if deployLoading}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
            Deploying from {deployBranch}…
          {:else}
            Deploy Now
          {/if}
        </button>
      </div>
    </div>

    <!-- Deploy History -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-5 py-4 border-b border-border flex items-center justify-between">
        <h3 class="text-sm font-semibold text-foreground">Deploy History</h3>
        <button class="text-xs text-muted-foreground hover:text-foreground transition-colors" on:click={() => loadHistory()}>Refresh</button>
      </div>

      {#if historyLoading && history.length === 0}
        <div class="p-5 space-y-3">
          {#each [1,2,3] as _}
            <div class="h-10 bg-muted rounded-lg animate-pulse"></div>
          {/each}
        </div>
      {:else if history.length === 0}
        <div class="p-10 text-center text-sm text-muted-foreground">
          No deployments yet. Push to your repo or click Deploy Now.
        </div>
      {:else}
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="border-b border-border bg-muted/30">
                <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">Commit</th>
                <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">Message</th>
                <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">Deployed By</th>
                <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">Date</th>
                <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">Duration</th>
                <th class="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground">Status</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-border">
              {#each history as entry (entry.id)}
                <tr class="hover:bg-muted/20 transition-colors">
                  <td class="px-4 py-3">
                    <code class="bg-muted px-1.5 py-0.5 rounded text-xs font-mono">{shortHash(entry.commit_hash)}</code>
                  </td>
                  <td class="px-4 py-3 max-w-xs">
                    <span class="text-foreground truncate block">{entry.commit_message}</span>
                    {#if entry.status === 'failed' && entry.error}
                      <button
                        class="text-xs text-red-400 hover:text-red-300 mt-0.5"
                        on:click={() => expandedErrors[entry.id] = !expandedErrors[entry.id]}
                      >
                        {expandedErrors[entry.id] ? 'Hide Error' : 'View Error'}
                      </button>
                      {#if expandedErrors[entry.id]}
                        <pre class="mt-2 bg-red-500/10 border border-red-500/20 rounded p-2 text-xs text-red-300 whitespace-pre-wrap max-w-sm">{entry.error}</pre>
                      {/if}
                    {/if}
                  </td>
                  <td class="px-4 py-3 text-muted-foreground">{entry.deployed_by}</td>
                  <td class="px-4 py-3 text-muted-foreground whitespace-nowrap">{relTime(entry.deployed_at)}</td>
                  <td class="px-4 py-3 text-muted-foreground">{entry.duration_ms != null ? `${entry.duration_ms}ms` : '—'}</td>
                  <td class="px-4 py-3">
                    {#if entry.status === 'success'}
                      <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-green-500/10 text-green-400">
                        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/></svg>
                        Success
                      </span>
                    {:else if entry.status === 'failed'}
                      <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-red-500/10 text-red-400">
                        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5"><path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/></svg>
                        Failed
                      </span>
                    {:else}
                      <span class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium bg-blue-500/10 text-blue-400">
                        <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
                        Running
                      </span>
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
        {#if history.length >= historyPage * PAGE_SIZE}
          <div class="px-5 py-3 border-t border-border">
            <button
              class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
              disabled={historyLoading}
              on:click={loadMore}
            >
              {historyLoading ? 'Loading…' : 'Load more'}
            </button>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<!-- ── Disable confirm modal ──────────────────────────────────────────────── -->
{#if showDisableConfirm}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={() => showDisableConfirm = false}
  >
    <div class="bg-card border border-border rounded-xl p-6 w-full max-w-sm space-y-4 shadow-2xl">
      <h2 class="text-base font-semibold text-foreground">Disable Git Deploy?</h2>
      <p class="text-sm text-muted-foreground">
        This will remove the deploy key and webhook. Future pushes will not trigger deployments. Deploy history will be preserved.
      </p>
      <div class="flex justify-end gap-3">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={() => showDisableConfirm = false}
        >Cancel</button>
        <button
          class="h-9 px-4 rounded-lg bg-red-500 text-white text-sm font-medium hover:bg-red-600 transition-colors disabled:opacity-60"
          disabled={disableLoading}
          on:click={disableGitDeploy}
        >
          {disableLoading ? 'Disabling…' : 'Disable'}
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
