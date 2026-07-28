<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$api/client';
  import type { Backup, BackupSettings, Site } from '$api/client';

  // ── Props ──────────────────────────────────────────────────────────────────
  export let siteId: string;
  export let site: Site;

  // ── Types ──────────────────────────────────────────────────────────────────
  interface Toast {
    id: number;
    message: string;
    type: 'success' | 'error';
  }

  // ── State ──────────────────────────────────────────────────────────────────
  let backups: Backup[] = [];
  let settings: BackupSettings | null = null;
  let loading = true;
  let settingsSaving = false;
  let backupCreating: string | null = null;
  let showRestoreConfirm: Backup | null = null;
  let restoreConfirmed = false;
  let restoring = false;
  let pollInterval: ReturnType<typeof setInterval> | null = null;

  // Settings form state
  let bkDailyTime = '03:00';
  let bkRetentionDays = 30;
  let bkDestination = 'local';
  let bkS3Bucket = '';
  let bkS3Region = 'us-east-1';
  let bkS3Key = '';
  let bkS3Secret = '';
  let bkSftpHost = '';
  let bkSftpUser = '';
  let bkSftpPath = '';

  // Inline delete confirmation
  let confirmDeleteId: string | null = null;

  // Toast queue
  let toasts: Toast[] = [];
  let toastCounter = 0;

  // ── Helpers ────────────────────────────────────────────────────────────────
  function formatBytes(b: number | null): string {
    if (!b) return '—';
    if (b < 1024) return b + ' B';
    if (b < 1048576) return (b / 1024).toFixed(1) + ' KB';
    if (b < 1073741824) return (b / 1048576).toFixed(1) + ' MB';
    return (b / 1073741824).toFixed(1) + ' GB';
  }

  function formatDate(iso: string | null): string {
    if (!iso) return '—';
    return new Date(iso).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  }

  function timeSince(iso: string): string {
    const diff = Date.now() - new Date(iso).getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 60) return `${mins}m ago`;
    const hrs = Math.floor(mins / 60);
    if (hrs < 24) return `${hrs}h ago`;
    return `${Math.floor(hrs / 24)}d ago`;
  }

  function showToast(message: string, type: 'success' | 'error' = 'success') {
    const id = ++toastCounter;
    toasts = [...toasts, { id, message, type }];
    setTimeout(() => { toasts = toasts.filter(t => t.id !== id); }, 4000);
  }

  function hasActiveBackups(list: Backup[]): boolean {
    return list.some(b => b.status === 'running' || b.status === 'pending');
  }

  function startPolling() {
    if (pollInterval) return;
    pollInterval = setInterval(async () => {
      try {
        backups = await api.backups.list(siteId);
        if (!hasActiveBackups(backups)) stopPolling();
      } catch { /* silently ignore poll errors */ }
    }, 5000);
  }

  function stopPolling() {
    if (pollInterval) {
      clearInterval(pollInterval);
      pollInterval = null;
    }
  }

  function applySettings(s: BackupSettings) {
    bkDailyTime = s.daily_time ?? '03:00';
    bkRetentionDays = s.retention_days ?? 30;
    bkDestination = s.destination ?? 'local';
    bkS3Bucket = s.s3_bucket ?? '';
    bkS3Region = s.s3_region ?? 'us-east-1';
    bkS3Key = '';
    bkS3Secret = '';
    bkSftpHost = s.sftp_host ?? '';
    bkSftpUser = s.sftp_user ?? '';
    bkSftpPath = s.sftp_path ?? `/backups/${site?.domain ?? 'site'}`;
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    try {
      const [bkList, bkSettings] = await Promise.all([
        api.backups.list(siteId),
        api.backups.getSettings(siteId),
      ]);
      backups = bkList;
      settings = bkSettings;
      applySettings(bkSettings);
      if (hasActiveBackups(backups)) startPolling();
    } catch {
      showToast('Failed to load backup data', 'error');
    } finally {
      loading = false;
    }
  });

  onDestroy(() => { stopPolling(); });

  // ── Actions ────────────────────────────────────────────────────────────────
  async function createBackup(type: 'full' | 'files_only' | 'db_only') {
    backupCreating = type;
    try {
      const bk = await api.backups.create(siteId, type);
      backups = [bk, ...backups];
      showToast('Backup started');
      startPolling();
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to start backup', 'error');
    } finally {
      backupCreating = null;
    }
  }

  async function refreshBackups() {
    try {
      backups = await api.backups.list(siteId);
      if (hasActiveBackups(backups)) startPolling();
    } catch {
      showToast('Failed to refresh backups', 'error');
    }
  }

  async function saveSettings() {
    settingsSaving = true;
    try {
      await api.backups.updateSettings(siteId, {
        daily_time: bkDailyTime,
        retention_days: bkRetentionDays,
        destination: bkDestination,
        s3_bucket: bkDestination === 's3' ? bkS3Bucket : undefined,
        s3_region: bkDestination === 's3' ? bkS3Region : undefined,
        s3_access_key: bkDestination === 's3' && bkS3Key ? bkS3Key : undefined,
        s3_secret_key: bkDestination === 's3' && bkS3Secret ? bkS3Secret : undefined,
        sftp_host: bkDestination === 'sftp' ? bkSftpHost : undefined,
        sftp_user: bkDestination === 'sftp' ? bkSftpUser : undefined,
        sftp_path: bkDestination === 'sftp' ? bkSftpPath : undefined,
      });
      showToast('Settings saved');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to save settings', 'error');
    } finally {
      settingsSaving = false;
    }
  }

  function openRestore(bk: Backup) {
    showRestoreConfirm = bk;
    restoreConfirmed = false;
  }

  function closeRestore() {
    showRestoreConfirm = null;
    restoreConfirmed = false;
  }

  async function handleRestore() {
    if (!showRestoreConfirm || !restoreConfirmed) return;
    restoring = true;
    try {
      await api.backups.restore(showRestoreConfirm.id);
      closeRestore();
      showToast('Restore started. This may take several minutes.');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Restore failed', 'error');
    } finally {
      restoring = false;
    }
  }

  async function handleDelete(backupId: string) {
    try {
      await api.backups.delete(backupId);
      backups = backups.filter(b => b.id !== backupId);
      confirmDeleteId = null;
      showToast('Backup deleted');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to delete backup', 'error');
    }
  }

  function handleDownload(backupId: string) {
    const url = api.backups.download(backupId);
    window.location.href = url;
  }

  function closeOnEsc(e: KeyboardEvent) {
    if (e.key === 'Escape') closeRestore();
  }

  // ── Badge classes ──────────────────────────────────────────────────────────
  const typeBadge: Record<string, string> = {
    full:       'bg-blue-500/10 text-blue-400 border border-blue-500/20',
    files_only: 'bg-green-500/10 text-green-400 border border-green-500/20',
    db_only:    'bg-purple-500/10 text-purple-400 border border-purple-500/20',
  };
  const typeLabel: Record<string, string> = {
    full: 'Full', files_only: 'Files Only', db_only: 'DB Only',
  };

  const statusBadge: Record<string, string> = {
    pending:   'bg-yellow-500/10 text-yellow-400 border border-yellow-500/20',
    running:   'bg-blue-500/10 text-blue-400 border border-blue-500/20',
    completed: 'bg-green-500/10 text-green-400 border border-green-500/20',
    failed:    'bg-red-500/10 text-red-400 border border-red-500/20',
    expired:   'bg-muted text-muted-foreground border border-border',
  };

  const S3_REGIONS = [
    'us-east-1', 'us-east-2', 'us-west-1', 'us-west-2',
    'eu-west-1', 'eu-west-2', 'eu-central-1',
    'ap-southeast-1', 'ap-southeast-2', 'ap-northeast-1',
    'sa-east-1', 'ca-central-1',
  ];
</script>

<svelte:window on:keydown={closeOnEsc} />

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  .card-hover { transition: all 0.2s ease; }
  .card-hover:hover { transform: translateY(-1px); box-shadow: 0 8px 30px rgba(0,0,0,0.2); }
  @keyframes progressPulse { 0%, 100% { opacity: 1 } 50% { opacity: 0.7 } }
  .progress-pulse { animation: progressPulse 1.5s ease-in-out infinite }
</style>

<!-- ── Two-column layout ────────────────────────────────────────────────────── -->
<div class="tab-content grid grid-cols-1 lg:grid-cols-2 gap-4 items-start">

  <!-- ── LEFT COLUMN ─────────────────────────────────────────────────────── -->
  <div class="space-y-4">

    <!-- Schedule Card -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-4 py-3 border-b border-border">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M12 6v6h4.5m4.5 0a9 9 0 11-18 0 9 9 0 0118 0z"/>
            </svg>
          </div>
          <span class="text-sm font-semibold text-foreground">Automated Backups</span>
        </div>
      </div>
      <div class="p-4 space-y-4">
        {#if loading}
          <div class="space-y-3">
            {#each [1, 2] as _}
              <div class="animate-pulse bg-muted rounded h-10"></div>
            {/each}
          </div>
        {:else}
          <div>
            <label class="block text-xs font-medium text-muted-foreground mb-1">Daily backup time</label>
            <input
              type="time"
              bind:value={bkDailyTime}
              class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
            />
          </div>
          <div>
            <label class="block text-xs font-medium text-muted-foreground mb-1">Retention period</label>
            <select
              bind:value={bkRetentionDays}
              class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
            >
              {#each [7, 14, 30, 60, 90] as d}
                <option value={d}>{d} days</option>
              {/each}
            </select>
          </div>
          <div class="flex justify-end">
            <button
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
              disabled={settingsSaving}
              on:click={saveSettings}
            >
              {settingsSaving ? 'Saving…' : 'Save Schedule'}
            </button>
          </div>
        {/if}
      </div>
    </div>

    <!-- Storage Destination Card -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-4 py-3 border-b border-border">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M20.25 6.375c0 2.278-3.694 4.125-8.25 4.125S3.75 8.653 3.75 6.375m16.5 0c0-2.278-3.694-4.125-8.25-4.125S3.75 4.097 3.75 6.375m16.5 0v11.25c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125V6.375m16.5 0v3.75m-16.5-3.75v3.75m16.5 0v3.75C20.25 16.153 16.556 18 12 18s-8.25-1.847-8.25-4.125v-3.75m16.5 0c0 2.278-3.694 4.125-8.25 4.125s-8.25-1.847-8.25-4.125"/>
            </svg>
          </div>
          <span class="text-sm font-semibold text-foreground">Backup Destination</span>
        </div>
      </div>
      <div class="p-4 space-y-3">
        {#if loading}
          <div class="space-y-2">
            {#each [1, 2, 3] as _}
              <div class="animate-pulse bg-muted rounded h-12"></div>
            {/each}
          </div>
        {:else}
          <!-- Destination radio cards -->
          {#each [
            { value: 'local', label: 'Local', desc: 'Stored on this server' },
            { value: 's3',    label: 'Amazon S3', desc: 'Offsite object storage' },
            { value: 'sftp',  label: 'SFTP', desc: 'Remote server via SFTP' },
          ] as opt}
            <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
            <label
              class="flex items-start gap-3 p-3 rounded-xl border cursor-pointer transition-colors {bkDestination === opt.value ? 'border-primary bg-primary/5' : 'border-border hover:bg-muted/20'}"
            >
              <input type="radio" bind:group={bkDestination} value={opt.value} class="mt-0.5 accent-primary" />
              <div class="flex-1 min-w-0">
                <p class="text-sm font-medium text-foreground">{opt.label}</p>
                <p class="text-xs text-muted-foreground">{opt.desc}</p>
              </div>
              {#if bkDestination === opt.value}
                <svg class="w-4 h-4 text-primary flex-shrink-0 mt-0.5" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z" />
                </svg>
              {/if}
            </label>
          {/each}

          <!-- S3 fields -->
          {#if bkDestination === 's3'}
            <div class="mt-2 space-y-3 pt-3 border-t border-border">
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1">Bucket name</label>
                <input
                  type="text"
                  bind:value={bkS3Bucket}
                  placeholder="my-backup-bucket"
                  class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                />
              </div>
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1">Region</label>
                <select
                  bind:value={bkS3Region}
                  class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                >
                  {#each S3_REGIONS as r}
                    <option value={r}>{r}</option>
                  {/each}
                </select>
              </div>
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1">Access Key ID</label>
                <input
                  type="text"
                  bind:value={bkS3Key}
                  placeholder="AKIAIOSFODNN7EXAMPLE"
                  class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
                />
              </div>
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1">Secret Access Key</label>
                <input
                  type="password"
                  bind:value={bkS3Secret}
                  placeholder="Leave blank to keep current"
                  class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
                />
              </div>
            </div>
          {/if}

          <!-- SFTP fields -->
          {#if bkDestination === 'sftp'}
            <div class="mt-2 space-y-3 pt-3 border-t border-border">
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1">Host</label>
                <input
                  type="text"
                  bind:value={bkSftpHost}
                  placeholder="backup.example.com"
                  class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                />
              </div>
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1">Username</label>
                <input
                  type="text"
                  bind:value={bkSftpUser}
                  placeholder="backupuser"
                  class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                />
              </div>
              <div>
                <label class="block text-xs font-medium text-muted-foreground mb-1">Remote path</label>
                <input
                  type="text"
                  bind:value={bkSftpPath}
                  placeholder="/backups/{site.domain}"
                  class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
                />
              </div>
            </div>
          {/if}

          <div class="flex justify-end pt-1">
            <button
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
              disabled={settingsSaving}
              on:click={saveSettings}
            >
              {settingsSaving ? 'Saving…' : 'Save Storage Settings'}
            </button>
          </div>
        {/if}
      </div>
    </div>
  </div>

  <!-- ── RIGHT COLUMN ─────────────────────────────────────────────────────── -->
  <div class="space-y-4">

    <!-- Manual Backup Card -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-4 py-3 border-b border-border">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M9 8.25H7.5a2.25 2.25 0 00-2.25 2.25v9a2.25 2.25 0 002.25 2.25h9a2.25 2.25 0 002.25-2.25v-9a2.25 2.25 0 00-2.25-2.25H15M9 12l3 3m0 0l3-3m-3 3V2.25"/>
            </svg>
          </div>
          <span class="text-sm font-semibold text-foreground">Create Backup</span>
        </div>
      </div>
      <div class="p-4">
        <p class="text-xs text-muted-foreground mb-3">Manually trigger a backup right now.</p>
        <div class="flex flex-wrap gap-2">
          <!-- Full Backup -->
          <button
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50 flex items-center gap-2"
            disabled={backupCreating !== null}
            on:click={() => createBackup('full')}
          >
            {#if backupCreating === 'full'}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8z"/>
              </svg>
              Creating…
            {:else}
              Full Backup
            {/if}
          </button>

          <!-- Files Only -->
          <button
            class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors disabled:opacity-50 flex items-center gap-2"
            disabled={backupCreating !== null}
            on:click={() => createBackup('files_only')}
          >
            {#if backupCreating === 'files_only'}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8z"/>
              </svg>
              Creating…
            {:else}
              Files Only
            {/if}
          </button>

          <!-- DB Only -->
          <button
            class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors disabled:opacity-50 flex items-center gap-2"
            disabled={backupCreating !== null}
            on:click={() => createBackup('db_only')}
          >
            {#if backupCreating === 'db_only'}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8z"/>
              </svg>
              Creating…
            {:else}
              Database Only
            {/if}
          </button>
        </div>
      </div>
    </div>

    <!-- Backup History Card -->
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="px-4 py-3 border-b border-border flex items-center justify-between">
        <div class="flex items-center gap-2.5">
          <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
            <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <path stroke-linecap="round" stroke-linejoin="round" d="M3.75 9.776c.112-.017.227-.026.344-.026h15.812c.117 0 .232.009.344.026m-16.5 0a2.25 2.25 0 00-1.883 2.542l.857 6a2.25 2.25 0 002.227 1.932H19.05a2.25 2.25 0 002.227-1.932l.857-6a2.25 2.25 0 00-1.883-2.542m-16.5 0V6A2.25 2.25 0 016 3.75h3.879a1.5 1.5 0 011.06.44l2.122 2.12a1.5 1.5 0 001.06.44H18A2.25 2.25 0 0120.25 9v.776"/>
            </svg>
          </div>
          <span class="text-sm font-semibold text-foreground">Backup History</span>
        </div>
        <button
          title="Refresh"
          class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors"
          on:click={refreshBackups}
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
          </svg>
        </button>
      </div>

      {#if loading}
        <div class="p-4 space-y-3">
          {#each [1, 2, 3] as _}
            <div class="flex items-center gap-3">
              <div class="animate-pulse bg-muted rounded h-6 w-20"></div>
              <div class="animate-pulse bg-muted rounded h-5 w-16"></div>
              <div class="animate-pulse bg-muted rounded h-5 flex-1"></div>
              <div class="animate-pulse bg-muted rounded h-5 w-24"></div>
            </div>
          {/each}
        </div>

      {:else if backups.length === 0}
        <div class="text-center py-12">
          <svg class="w-12 h-12 text-muted-foreground/30 mx-auto mb-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4" />
          </svg>
          <p class="text-sm font-medium text-muted-foreground">No backups yet</p>
          <p class="text-xs text-muted-foreground mt-1">Create your first backup using the buttons above</p>
        </div>

      {:else}
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead>
              <tr class="bg-muted/30">
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Type</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Size</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Status</th>
                <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Created</th>
                <th class="text-right px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Actions</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-border">
              {#each backups as bk (bk.id)}
                <tr class="hover:bg-muted/10 transition-colors">
                  <!-- Type -->
                  <td class="px-4 py-3">
                    <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium {typeBadge[bk.type] ?? 'bg-muted text-muted-foreground'}">
                      {typeLabel[bk.type] ?? bk.type}
                    </span>
                  </td>

                  <!-- Size -->
                  <td class="px-4 py-3 text-xs text-muted-foreground font-mono">
                    {formatBytes(bk.size_bytes)}
                  </td>

                  <!-- Status -->
                  <td class="px-4 py-3">
                    {#if bk.status === 'running'}
                      <div>
                        <span class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium {statusBadge[bk.status] ?? 'bg-muted text-muted-foreground'} mb-1">
                          <span class="w-1.5 h-1.5 rounded-full bg-current animate-pulse"></span>
                          Running
                        </span>
                        <div class="h-1 bg-muted rounded-full overflow-hidden w-20">
                          <div class="h-full bg-blue-400 rounded-full progress-pulse" style="width: 60%"></div>
                        </div>
                      </div>
                    {:else}
                      <span class="inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium {statusBadge[bk.status] ?? 'bg-muted text-muted-foreground'}">
                        {#if bk.status === 'pending'}
                          <span class="w-1.5 h-1.5 rounded-full bg-current animate-pulse"></span>
                        {/if}
                        {bk.status.charAt(0).toUpperCase() + bk.status.slice(1)}
                      </span>
                    {/if}
                  </td>

                  <!-- Created -->
                  <td class="px-4 py-3 text-xs text-muted-foreground" title={formatDate(bk.created_at)}>
                    {timeSince(bk.created_at)}
                  </td>

                  <!-- Actions -->
                  <td class="px-4 py-3">
                    <div class="flex items-center justify-end gap-1">

                      <!-- Download — only for completed -->
                      {#if bk.status === 'completed'}
                        <button
                          title="Download"
                          class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors"
                          on:click={() => handleDownload(bk.id)}
                        >
                          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                              d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                          </svg>
                        </button>

                        <!-- Restore — danger with shield icon -->
                        <button
                          title="Restore"
                          class="h-8 px-2 flex items-center justify-center rounded-lg text-amber-400 hover:bg-amber-500/10 hover:border-amber-500/30 border border-transparent transition-colors gap-1"
                          on:click={() => openRestore(bk)}
                        >
                          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M9 12.75L11.25 15 15 9.75m-3-7.036A11.959 11.959 0 013.598 6 11.99 11.99 0 003 9.749c0 5.592 3.824 10.29 9 11.623 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.571-.598-3.751h-.152c-3.196 0-6.1-1.248-8.25-3.285z"/>
                          </svg>
                          <span class="text-xs font-medium">Restore</span>
                        </button>
                      {/if}

                      <!-- Delete with inline confirm -->
                      {#if confirmDeleteId === bk.id}
                        <span class="text-xs text-muted-foreground mr-1">Sure?</span>
                        <button
                          class="h-7 px-2 rounded text-xs font-medium text-red-400 hover:bg-red-500/10 transition-colors"
                          on:click={() => handleDelete(bk.id)}
                        >
                          Yes
                        </button>
                        <button
                          class="h-7 px-2 rounded text-xs font-medium text-muted-foreground hover:bg-muted transition-colors"
                          on:click={() => { confirmDeleteId = null; }}
                        >
                          No
                        </button>
                      {:else}
                        <button
                          title="Delete"
                          class="h-8 w-8 flex items-center justify-center rounded-lg text-red-400 hover:bg-red-500/10 transition-colors"
                          on:click={() => { confirmDeleteId = bk.id; }}
                        >
                          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                              d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                          </svg>
                        </button>
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
  </div>
</div>


<!-- ── Restore Confirmation Modal ────────────────────────────────────────────── -->
{#if showRestoreConfirm}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeRestore}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Restore Backup</h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={closeRestore}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <!-- Backup summary -->
      <div class="flex items-center gap-3 mb-4 p-3 rounded-lg bg-muted/30 border border-border">
        <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium {typeBadge[showRestoreConfirm.type] ?? 'bg-muted text-muted-foreground'}">
          {typeLabel[showRestoreConfirm.type] ?? showRestoreConfirm.type}
        </span>
        <span class="text-sm text-muted-foreground">{formatDate(showRestoreConfirm.created_at)}</span>
        <span class="text-sm text-muted-foreground ml-auto">{formatBytes(showRestoreConfirm.size_bytes)}</span>
      </div>

      <!-- Warning -->
      <div class="bg-yellow-500/10 border border-yellow-500/20 rounded-lg p-3 text-sm text-yellow-300 mb-4">
        ⚠️ This will overwrite all files and databases for <strong>{site.domain}</strong> with the selected backup. This cannot be undone.
      </div>

      <!-- Confirm checkbox -->
      <label class="flex items-start gap-3 cursor-pointer mb-5">
        <input
          type="checkbox"
          bind:checked={restoreConfirmed}
          class="mt-0.5 rounded border-border accent-primary"
        />
        <span class="text-sm text-foreground">I understand this action is irreversible</span>
      </label>

      <div class="flex justify-end gap-2">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={closeRestore}
        >
          Cancel
        </button>
        <button
          class="h-9 px-4 rounded-lg text-sm font-medium bg-red-500 text-white hover:bg-red-600 transition-colors disabled:opacity-50"
          disabled={!restoreConfirmed || restoring}
          on:click={handleRestore}
        >
          {restoring ? 'Restoring…' : 'Restore Now'}
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Toast notifications ───────────────────────────────────────────────────── -->
<div class="fixed bottom-5 right-5 z-[60] flex flex-col gap-2 pointer-events-none">
  {#each toasts as toast (toast.id)}
    <div
      class="pointer-events-auto flex items-center gap-3 px-4 py-3 rounded-xl shadow-lg border text-sm font-medium
        {toast.type === 'success'
          ? 'bg-card border-green-500/30 text-foreground'
          : 'bg-card border-red-500/30 text-foreground'}"
    >
      {#if toast.type === 'success'}
        <svg class="w-4 h-4 text-green-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
        </svg>
      {:else}
        <svg class="w-4 h-4 text-red-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
      {/if}
      <span>{toast.message}</span>
    </div>
  {/each}
</div>
