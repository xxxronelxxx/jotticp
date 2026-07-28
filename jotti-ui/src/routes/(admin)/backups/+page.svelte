<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$api/client';
  import type { Backup, BackupSettings, Site } from '$api/client';

  // ── State ──────────────────────────────────────────────────────────────────

  let backups: Backup[] = [];
  let sites: Site[] = [];
  let loading = true;

  // Stats
  let totalSizeBytes = 0;
  let successRate = 0;

  // Schedules
  interface ScheduleRow {
    siteId:   string;
    domain:   string;
    settings: BackupSettings | null;
    loading:  boolean;
  }
  let schedules: ScheduleRow[] = [];

  // History view toggle
  let historyView: 'table' | 'timeline' = 'table';

  // Modals
  let showAddSchedule   = false;
  let showRestoreModal  = false;
  let showStorageConfig = false;

  // Add schedule form
  let sched_siteId      = '';
  let sched_frequency   = 'daily';
  let sched_time        = '03:00';
  let sched_retentionD  = 7;
  let sched_retentionW  = 4;
  let sched_saving      = false;

  // Restore 2-step modal state
  let restoreTarget: Backup | null = null;
  let restoreStep: 1 | 2 = 1;
  let restoreConfirmDomain = '';
  let restoreFiles = true;
  let restoreDatabase = true;
  let restoring = false;

  // Storage config
  let storage_destination = 'local';
  let storage_siteId      = '';
  let storage_s3_bucket   = '';
  let storage_s3_region   = '';
  let storage_sftp_host   = '';
  let storage_sftp_user   = '';
  let storage_sftp_path   = '';
  let storage_saving      = false;

  // Run now tracking
  let runningNow = new Set<string>();

  // Backup progress tracking
  interface BackupProgress {
    siteId:    string;
    startedAt: number;
    step:      number; // 0-5
  }
  let activeProgress: BackupProgress | null = null;
  let progressInterval: ReturnType<typeof setInterval> | null = null;

  const BACKUP_STEPS = ['Preparing', 'Files', 'Database', 'Compressing', 'Uploading', 'Done'];
  const TYPICAL_DURATION_MS = 90_000; // 90 seconds typical

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  onMount(async () => {
    loading = true;
    try {
      const [backupList, siteList] = await Promise.all([
        api.backups.listAll(),
        api.sites.list(),
      ]);
      backups = backupList;
      sites   = siteList;

      totalSizeBytes = backups.reduce((sum, b) => sum + (b.size_bytes ?? 0), 0);
      const last30 = backups.filter(b => {
        if (!b.created_at) return false;
        return Date.now() - new Date(b.created_at).getTime() < 30 * 86_400_000;
      });
      const completed30 = last30.filter(b => b.status === 'completed').length;
      successRate = last30.length > 0 ? Math.round((completed30 / last30.length) * 100) : 0;

      schedules = sites.map(s => ({ siteId: s.id, domain: s.domain, settings: null, loading: false }));
      if (schedules.length > 0) {
        sched_siteId   = schedules[0].siteId;
        storage_siteId = schedules[0].siteId;
      }

      schedules.forEach(async (row, idx) => {
        schedules[idx].loading = true;
        try {
          schedules[idx].settings = await api.backups.getSettings(row.siteId);
        } catch { /* no settings yet */ }
        schedules[idx].loading = false;
        schedules = [...schedules];
      });
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load backup data';
    } finally {
      loading = false;
    }
  });

  onDestroy(() => {
    if (progressInterval) clearInterval(progressInterval);
  });

  // ── Derived ────────────────────────────────────────────────────────────────

  $: lastBackup = backups.length > 0
    ? backups.sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime())[0]
    : null;

  $: lastBackupAge = lastBackup ? Date.now() - new Date(lastBackup.created_at).getTime() : null;

  $: lastBackupColor = !lastBackupAge
    ? 'text-muted-foreground'
    : lastBackupAge < 86_400_000
      ? 'text-green-400'
      : lastBackupAge < 7 * 86_400_000
        ? 'text-amber-400'
        : 'text-red-400';

  $: recentHistory = [...backups]
    .sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
    .slice(0, 50);

  // Group by date for timeline view
  $: groupedByDate = (() => {
    const groups: { date: string; backups: Backup[] }[] = [];
    const seen = new Map<string, number>();
    for (const b of recentHistory) {
      if (!b.created_at) continue;
      const d = new Date(b.created_at);
      const key = d.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric' });
      if (seen.has(key)) {
        groups[seen.get(key)!].backups.push(b);
      } else {
        seen.set(key, groups.length);
        groups.push({ date: key, backups: [b] });
      }
    }
    return groups;
  })();

  // Last 7 days backup sizes for chart
  $: last7Days = (() => {
    const days: { label: string; shortLabel: string; size: number }[] = [];
    const now = new Date();
    for (let i = 6; i >= 0; i--) {
      const d = new Date(now);
      d.setDate(d.getDate() - i);
      d.setHours(0, 0, 0, 0);
      const end = new Date(d); end.setDate(end.getDate() + 1);
      const dayBackups = backups.filter(b => {
        if (!b.created_at) return false;
        const t = new Date(b.created_at).getTime();
        return t >= d.getTime() && t < end.getTime() && b.status === 'completed';
      });
      const size = dayBackups.reduce((s, b) => s + (b.size_bytes ?? 0), 0);
      days.push({
        label: d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }),
        shortLabel: d.toLocaleDateString(undefined, { weekday: 'short' }).slice(0, 2),
        size,
      });
    }
    return days;
  })();

  $: maxChartSize = Math.max(...last7Days.map(d => d.size), 1);

  // Backup progress computed values
  $: progressElapsed = activeProgress ? Date.now() - activeProgress.startedAt : 0;
  $: progressPct = activeProgress
    ? Math.min(Math.round((progressElapsed / TYPICAL_DURATION_MS) * 100), 95)
    : 0;
  $: progressStep = activeProgress ? activeProgress.step : 0;
  $: progressETA = activeProgress
    ? Math.max(0, Math.round((TYPICAL_DURATION_MS - progressElapsed) / 1000))
    : 0;

  // SVG ring progress
  $: ringCircumference = 2 * Math.PI * 20; // r=20
  $: ringOffset = ringCircumference - (progressPct / 100) * ringCircumference;

  // ── Actions ────────────────────────────────────────────────────────────────

  async function runBackupNow() {
    if (sites.length === 0) { showToast('No sites found', 'error'); return; }
    const siteId = sites[0].id;
    startProgressAnimation(siteId);
    runningNow.add(siteId);
    runningNow = new Set(runningNow);
    try {
      await api.backups.create(siteId, 'full');
      showToast('Backup started', 'success');
      const updated = await api.backups.listAll();
      backups = updated;
    } catch (e: any) {
      showToast(e.message || 'Failed to start backup', 'error');
      stopProgressAnimation();
    } finally {
      runningNow.delete(siteId);
      runningNow = new Set(runningNow);
    }
  }

  async function runSiteBackup(siteId: string) {
    startProgressAnimation(siteId);
    runningNow.add(siteId);
    runningNow = new Set(runningNow);
    try {
      await api.backups.create(siteId, 'full');
      showToast('Backup queued', 'success');
      backups = await api.backups.listAll();
    } catch (e: any) {
      showToast(e.message || 'Failed to start backup', 'error');
      stopProgressAnimation();
    } finally {
      runningNow.delete(siteId);
      runningNow = new Set(runningNow);
    }
  }

  function startProgressAnimation(siteId: string) {
    if (progressInterval) clearInterval(progressInterval);
    activeProgress = { siteId, startedAt: Date.now(), step: 0 };
    progressInterval = setInterval(() => {
      if (!activeProgress) return;
      const elapsed = Date.now() - activeProgress.startedAt;
      // Advance step every ~15s
      const newStep = Math.min(Math.floor(elapsed / 15000), BACKUP_STEPS.length - 2);
      activeProgress = { ...activeProgress, step: newStep };
      // Auto-stop after typical duration + 30s buffer
      if (elapsed > TYPICAL_DURATION_MS + 30_000) stopProgressAnimation();
    }, 500);
  }

  function stopProgressAnimation() {
    if (progressInterval) { clearInterval(progressInterval); progressInterval = null; }
    activeProgress = null;
  }

  async function saveSchedule() {
    if (!sched_siteId) return;
    sched_saving = true;
    try {
      await api.backups.updateSettings(sched_siteId, {
        daily_time:      sched_time,
        retention_days:  sched_retentionD,
        retention_weeks: sched_retentionW,
        destination:     'local',
      });
      showToast('Schedule saved', 'success');
      showAddSchedule = false;
      const idx = schedules.findIndex(r => r.siteId === sched_siteId);
      if (idx >= 0) {
        schedules[idx].settings = await api.backups.getSettings(sched_siteId);
        schedules = [...schedules];
      }
    } catch (e: any) {
      showToast(e.message || 'Failed to save schedule', 'error');
    } finally {
      sched_saving = false;
    }
  }

  function openRestoreModal(backup: Backup) {
    restoreTarget        = backup;
    restoreConfirmDomain = '';
    restoreStep          = 1;
    restoreFiles         = true;
    restoreDatabase      = true;
    showRestoreModal     = true;
  }

  async function confirmRestore() {
    if (!restoreTarget) return;
    const domain = sites.find(s => s.id === restoreTarget!.site_id)?.domain ?? '';
    if (restoreConfirmDomain !== domain) {
      showToast('Domain does not match', 'error');
      return;
    }
    restoring = true;
    try {
      await api.backups.restore(restoreTarget.id);
      showToast('Restore job queued', 'success');
      showRestoreModal = false;
    } catch (e: any) {
      showToast(e.message || 'Failed to start restore', 'error');
    } finally {
      restoring = false;
    }
  }

  async function saveStorageConfig() {
    if (!storage_siteId) return;
    storage_saving = true;
    try {
      await api.backups.updateSettings(storage_siteId, {
        destination: storage_destination,
        s3_bucket:   storage_s3_bucket || undefined,
        s3_region:   storage_s3_region || undefined,
        sftp_host:   storage_sftp_host || undefined,
        sftp_user:   storage_sftp_user || undefined,
        sftp_path:   storage_sftp_path || undefined,
      });
      showToast('Storage configuration saved', 'success');
      showStorageConfig = false;
    } catch (e: any) {
      showToast(e.message || 'Failed to save storage config', 'error');
    } finally {
      storage_saving = false;
    }
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType    = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  // ── Formatters ─────────────────────────────────────────────────────────────

  function formatBytes(bytes: number | null): string {
    if (!bytes) return '—';
    if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(1)} GB`;
    if (bytes >= 1_048_576)     return `${(bytes / 1_048_576).toFixed(1)} MB`;
    return `${Math.round(bytes / 1024)} KB`;
  }

  function timeAgo(dateStr: string | null): string {
    if (!dateStr) return 'Never';
    const diff = Date.now() - new Date(dateStr).getTime();
    const mins  = Math.floor(diff / 60_000);
    const hours = Math.floor(diff / 3_600_000);
    const days  = Math.floor(diff / 86_400_000);
    if (mins  < 1)  return 'Just now';
    if (mins  < 60) return `${mins}m ago`;
    if (hours < 24) return `${hours}h ago`;
    return `${days}d ago`;
  }

  function formatDateTime(dateStr: string | null): string {
    if (!dateStr) return '—';
    return new Date(dateStr).toLocaleString(undefined, {
      month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
    });
  }

  function scheduleLabel(settings: BackupSettings | null): string {
    if (!settings) return 'Not configured';
    return `Daily at ${settings.daily_time}`;
  }

  function nextRun(settings: BackupSettings | null): string {
    if (!settings) return '—';
    const [h, m] = settings.daily_time.split(':').map(Number);
    const next = new Date();
    next.setHours(h, m, 0, 0);
    if (next.getTime() < Date.now()) next.setDate(next.getDate() + 1);
    return next.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  }

  function statusBadgeClass(status: string): string {
    const map: Record<string, string> = {
      completed: 'bg-green-500/10 text-green-400 border border-green-500/20',
      running:   'bg-amber-500/10 text-amber-400 border border-amber-500/20',
      pending:   'bg-blue-500/10 text-blue-400 border border-blue-500/20',
      failed:    'bg-red-500/10 text-red-400 border border-red-500/20',
      expired:   'bg-muted text-muted-foreground border border-border',
    };
    return map[status] ?? 'bg-muted text-muted-foreground border border-border';
  }

  function formatSeconds(s: number): string {
    if (s < 60) return `${s}s`;
    return `${Math.floor(s / 60)}m ${s % 60}s`;
  }
</script>

<svelte:head>
  <title>Backups — JottiCP</title>
</svelte:head>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px) }
    to   { opacity: 1; transform: none }
  }
  @keyframes slideInRight {
    from { opacity: 0; transform: translateX(8px) }
    to   { opacity: 1; transform: none }
  }
  .fade-up        { animation: fadeUp 0.25s ease-out both }
  .slide-in-right { animation: slideInRight 0.2s ease-out both }
</style>

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
      <button on:click={() => window.location.reload()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- ── Header ──────────────────────────────────────────────────────────── -->
  <div class="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3 fade-up">
    <div>
      <h1 class="text-xl font-bold text-foreground">Backups</h1>
      <p class="text-sm text-muted-foreground mt-0.5">Manage backup schedules and restore points</p>
    </div>
    <div class="flex items-center gap-2">
      <button
        class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
        on:click={() => showStorageConfig = !showStorageConfig}
      >
        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826
               2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066
               2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924
               1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724
               1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31
               2.37-2.37.996.608 2.296.07 2.572-1.065z"/>
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
        </svg>
        Configure Storage
      </button>
      <button
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50"
        on:click={runBackupNow}
        disabled={loading || sites.length === 0 || !!activeProgress}
      >
        {#if activeProgress}
          <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
          </svg>
          Running…
        {:else}
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12"/>
          </svg>
          Run Backup Now
        {/if}
      </button>
    </div>
  </div>

  <!-- ── Stats row ───────────────────────────────────────────────────────── -->
  {#if loading}
    <div class="grid grid-cols-2 lg:grid-cols-4 gap-3">
      {#each [1,2,3,4] as _}
        <div class="h-24 bg-muted rounded-xl animate-pulse"></div>
      {/each}
    </div>
  {:else}
    <div class="grid grid-cols-2 lg:grid-cols-4 gap-3">
      <div class="bg-card border border-border rounded-xl p-4 fade-up" style="animation-delay:0.05s">
        <p class="text-xs text-muted-foreground uppercase tracking-wide font-medium mb-1">Last Backup</p>
        <p class="text-lg font-bold {lastBackupColor}">{timeAgo(lastBackup?.created_at ?? null)}</p>
        <p class="text-xs text-muted-foreground mt-0.5">{lastBackup ? formatDateTime(lastBackup.created_at) : 'No backups yet'}</p>
      </div>
      <div class="bg-card border border-border rounded-xl p-4 fade-up" style="animation-delay:0.08s">
        <p class="text-xs text-muted-foreground uppercase tracking-wide font-medium mb-1">Total Size</p>
        <p class="text-lg font-bold text-foreground">{formatBytes(totalSizeBytes)}</p>
        <p class="text-xs text-muted-foreground mt-0.5">{backups.length} backup{backups.length !== 1 ? 's' : ''} stored</p>
      </div>
      <div class="bg-card border border-border rounded-xl p-4 fade-up" style="animation-delay:0.11s">
        <p class="text-xs text-muted-foreground uppercase tracking-wide font-medium mb-1">Backup Jobs</p>
        <p class="text-lg font-bold text-foreground">{schedules.filter(s => s.settings).length}</p>
        <p class="text-xs text-muted-foreground mt-0.5">of {sites.length} sites configured</p>
      </div>
      <div class="bg-card border border-border rounded-xl p-4 fade-up" style="animation-delay:0.14s">
        <p class="text-xs text-muted-foreground uppercase tracking-wide font-medium mb-1">Success Rate</p>
        <p class="text-lg font-bold {successRate >= 90 ? 'text-green-400' : successRate >= 70 ? 'text-amber-400' : 'text-red-400'}">{successRate}%</p>
        <p class="text-xs text-muted-foreground mt-0.5">Last 30 days</p>
        <div class="bg-muted rounded-full h-1.5 mt-2">
          <div class="h-1.5 rounded-full transition-all {successRate >= 90 ? 'bg-green-500' : successRate >= 70 ? 'bg-amber-500' : 'bg-red-500'}"
               style="width:{successRate}%"></div>
        </div>
      </div>
    </div>
  {/if}

  <!-- ── Active backup progress ─────────────────────────────────────────── -->
  {#if activeProgress}
    <div class="bg-card border border-amber-500/20 rounded-xl p-5 fade-up">
      <div class="flex items-center gap-5">
        <!-- SVG progress ring -->
        <div class="shrink-0 relative w-16 h-16">
          <svg class="w-16 h-16 -rotate-90" viewBox="0 0 48 48">
            <circle cx="24" cy="24" r="20" fill="none" stroke="currentColor" stroke-width="4"
                    class="text-muted/30"/>
            <circle cx="24" cy="24" r="20" fill="none" stroke="currentColor" stroke-width="4"
                    class="text-amber-400 transition-all duration-500"
                    stroke-dasharray="{ringCircumference}"
                    stroke-dashoffset="{ringOffset}"
                    stroke-linecap="round"/>
          </svg>
          <span class="absolute inset-0 flex items-center justify-center text-xs font-bold text-amber-400">
            {progressPct}%
          </span>
        </div>

        <div class="flex-1 space-y-2">
          <div class="flex items-center justify-between">
            <p class="text-sm font-semibold text-foreground">
              Backup in progress — {sites.find(s => s.id === activeProgress?.siteId)?.domain ?? 'site'}
            </p>
            <button on:click={stopProgressAnimation}
                    class="text-xs text-muted-foreground hover:text-foreground transition-colors">
              Dismiss
            </button>
          </div>

          <!-- Step indicators -->
          <div class="flex items-center gap-1">
            {#each BACKUP_STEPS as step, i}
              <div class="flex items-center gap-1">
                <div class="flex items-center gap-1">
                  <div class="w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-bold
                    {i < progressStep ? 'bg-green-500 text-white'
                     : i === progressStep ? 'bg-amber-400 text-black animate-pulse'
                     : 'bg-muted text-muted-foreground'}">
                    {#if i < progressStep}
                      <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="3" d="M5 13l4 4L19 7"/>
                      </svg>
                    {:else}
                      {i + 1}
                    {/if}
                  </div>
                  <span class="text-xs {i === progressStep ? 'text-amber-400 font-medium' : i < progressStep ? 'text-green-400' : 'text-muted-foreground'} hidden sm:inline">
                    {step}
                  </span>
                </div>
                {#if i < BACKUP_STEPS.length - 1}
                  <div class="w-4 h-px {i < progressStep ? 'bg-green-500' : 'bg-border'} mx-1"></div>
                {/if}
              </div>
            {/each}
          </div>

          <!-- Timers -->
          <div class="flex items-center gap-4 text-xs text-muted-foreground">
            <span>Elapsed: <span class="text-foreground font-medium">{formatSeconds(Math.round(progressElapsed / 1000))}</span></span>
            <span>ETA: <span class="text-foreground font-medium">{progressETA > 0 ? formatSeconds(progressETA) : 'Almost done'}</span></span>
            <span class="flex items-center gap-1 text-amber-400">
              <span class="w-1.5 h-1.5 rounded-full bg-amber-400 animate-pulse"></span>
              {BACKUP_STEPS[progressStep]}
            </span>
          </div>
        </div>
      </div>
    </div>
  {/if}

  <!-- ── Storage Configuration (collapsible) ─────────────────────────────── -->
  {#if showStorageConfig}
    <div class="bg-card border border-border rounded-xl p-5 space-y-4 slide-in-right">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-foreground">Storage Configuration</h2>
        <button class="text-muted-foreground hover:text-foreground" on:click={() => showStorageConfig = false}>
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Site</label>
          <select bind:value={storage_siteId}
                  class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
            {#each sites as s}<option value={s.id}>{s.domain}</option>{/each}
          </select>
        </div>
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Destination</label>
          <select bind:value={storage_destination}
                  class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
            <option value="local">Local disk</option>
            <option value="s3">Amazon S3 / S3-compatible</option>
            <option value="b2">Backblaze B2</option>
            <option value="sftp">FTP / SFTP</option>
          </select>
        </div>
      </div>

      {#if storage_destination === 's3' || storage_destination === 'b2'}
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div class="space-y-1">
            <label class="text-xs font-medium text-muted-foreground">Bucket name</label>
            <input bind:value={storage_s3_bucket} placeholder="my-backup-bucket"
                   class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
          </div>
          <div class="space-y-1">
            <label class="text-xs font-medium text-muted-foreground">Region</label>
            <input bind:value={storage_s3_region} placeholder="us-east-1"
                   class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
          </div>
        </div>
      {/if}

      {#if storage_destination === 'sftp'}
        <div class="grid grid-cols-1 sm:grid-cols-3 gap-4">
          <div class="space-y-1">
            <label class="text-xs font-medium text-muted-foreground">Host</label>
            <input bind:value={storage_sftp_host} placeholder="ftp.example.com"
                   class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
          </div>
          <div class="space-y-1">
            <label class="text-xs font-medium text-muted-foreground">Username</label>
            <input bind:value={storage_sftp_user} placeholder="backupuser"
                   class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
          </div>
          <div class="space-y-1">
            <label class="text-xs font-medium text-muted-foreground">Remote path</label>
            <input bind:value={storage_sftp_path} placeholder="/backups"
                   class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
          </div>
        </div>
      {/if}

      <div class="flex gap-2 pt-1">
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50"
          on:click={saveStorageConfig}
          disabled={storage_saving}
        >
          {storage_saving ? 'Saving…' : 'Save Configuration'}
        </button>
        <button class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
                on:click={() => showStorageConfig = false}>
          Cancel
        </button>
      </div>
    </div>
  {/if}

  <!-- ── Two-column layout ───────────────────────────────────────────────── -->
  <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">

    <!-- LEFT: Backup Jobs / Schedules -->
    <div class="space-y-4 fade-up" style="animation-delay:0.1s">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-foreground">Backup Schedules</h2>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2"
          on:click={() => { showAddSchedule = true; }}
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
          </svg>
          Add Schedule
        </button>
      </div>

      {#if loading}
        <div class="space-y-2">
          {#each [1,2,3] as _}
            <div class="h-16 bg-muted rounded-xl animate-pulse"></div>
          {/each}
        </div>
      {:else if schedules.length === 0}
        <div class="bg-card border border-border rounded-xl p-10 text-center">
          <svg class="w-10 h-10 text-muted-foreground mx-auto mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"/>
          </svg>
          <p class="font-medium text-foreground text-sm">No sites yet</p>
          <p class="text-xs text-muted-foreground mt-1">Add a site first, then configure backup schedules.</p>
        </div>
      {:else}
        <div class="bg-card border border-border rounded-xl overflow-hidden">
          <table class="w-full">
            <thead class="bg-muted/50">
              <tr>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase">Site</th>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase hidden sm:table-cell">Schedule</th>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase hidden md:table-cell">Next run</th>
                <th class="px-4 py-3 text-right text-xs font-medium text-muted-foreground uppercase">Actions</th>
              </tr>
            </thead>
            <tbody>
              {#each schedules as row (row.siteId)}
                <tr class="border-t border-border hover:bg-muted/30 transition-colors">
                  <td class="px-4 py-3">
                    <p class="text-sm font-medium text-foreground truncate max-w-[160px]">{row.domain}</p>
                    {#if row.loading}
                      <p class="text-xs text-muted-foreground animate-pulse">Loading…</p>
                    {:else}
                      <p class="text-xs text-muted-foreground">{scheduleLabel(row.settings)}</p>
                    {/if}
                  </td>
                  <td class="px-4 py-3 text-sm text-muted-foreground hidden sm:table-cell">
                    {#if row.settings}
                      <span class="bg-primary/10 text-primary border border-primary/20 px-2 py-0.5 rounded-full text-xs font-medium">
                        {scheduleLabel(row.settings)}
                      </span>
                    {:else}
                      <span class="text-xs text-muted-foreground">—</span>
                    {/if}
                  </td>
                  <td class="px-4 py-3 text-xs text-muted-foreground hidden md:table-cell">
                    {nextRun(row.settings)}
                  </td>
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-1 justify-end">
                      <button
                        class="h-8 px-3 rounded-lg bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90 inline-flex items-center gap-1 disabled:opacity-50"
                        on:click={() => runSiteBackup(row.siteId)}
                        disabled={runningNow.has(row.siteId) || !!activeProgress}
                      >
                        {#if runningNow.has(row.siteId)}
                          <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
                          </svg>
                        {:else}
                          Run
                        {/if}
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    </div>

    <!-- RIGHT: Backup History -->
    <div class="space-y-4 fade-up" style="animation-delay:0.15s">
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-foreground">Backup History</h2>
        <!-- View toggle -->
        {#if recentHistory.length > 0}
          <div class="flex items-center gap-1 bg-muted rounded-lg p-1">
            <button
              on:click={() => historyView = 'table'}
              class="h-7 px-3 rounded-md text-xs font-medium transition-colors
                {historyView === 'table' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
            >
              <svg class="w-3.5 h-3.5 inline mr-1" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 10h18M3 14h18M10 3v18M14 3v18"/>
              </svg>
              Table
            </button>
            <button
              on:click={() => historyView = 'timeline'}
              class="h-7 px-3 rounded-md text-xs font-medium transition-colors
                {historyView === 'timeline' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
            >
              <svg class="w-3.5 h-3.5 inline mr-1" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2"/>
              </svg>
              Timeline
            </button>
          </div>
        {/if}
      </div>

      <!-- Size history chart (always shown when data exists) -->
      {#if !loading && recentHistory.length > 0}
        <div class="bg-card border border-border rounded-xl p-4">
          <p class="text-xs font-medium text-muted-foreground mb-3">Backup size — last 7 days</p>
          <div class="flex items-end gap-1 h-12">
            {#each last7Days as day}
              <div class="flex-1 flex flex-col items-center gap-0.5">
                <div class="relative group w-full">
                  <div class="w-full bg-primary/20 rounded-sm hover:bg-primary/50 transition-colors cursor-pointer"
                       style="height: {day.size > 0 ? Math.max(4, (day.size / maxChartSize) * 36) : 2}px">
                  </div>
                  <!-- Tooltip -->
                  {#if day.size > 0}
                    <div class="absolute bottom-full mb-1 left-1/2 -translate-x-1/2 bg-card border border-border
                                text-xs text-foreground px-2 py-1 rounded-lg whitespace-nowrap opacity-0
                                group-hover:opacity-100 pointer-events-none z-10 shadow-lg">
                      {day.label}: {formatBytes(day.size)}
                    </div>
                  {/if}
                </div>
                <span class="text-[9px] text-muted-foreground">{day.shortLabel}</span>
              </div>
            {/each}
          </div>
        </div>
      {/if}

      {#if loading}
        <div class="space-y-2">
          {#each [1,2,3,4,5] as _}
            <div class="h-16 bg-muted rounded-xl animate-pulse"></div>
          {/each}
        </div>
      {:else if recentHistory.length === 0}
        <div class="bg-card border border-border rounded-xl p-10 text-center">
          <svg class="w-10 h-10 text-muted-foreground mx-auto mb-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21
                 3.582-4 8-4s8 1.79 8 4"/>
          </svg>
          <p class="font-medium text-foreground text-sm">No backups yet</p>
          <p class="text-xs text-muted-foreground mt-1">Run your first backup using the button above.</p>
          <button
            class="mt-4 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2"
            on:click={runBackupNow}
          >
            Run Backup Now
          </button>
        </div>

      {:else if historyView === 'table'}
        <!-- Table view -->
        <div class="bg-card border border-border rounded-xl overflow-hidden">
          <div class="overflow-x-auto">
            <table class="w-full">
              <thead class="bg-muted/50">
                <tr>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase">When</th>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase hidden sm:table-cell">Site</th>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase">Status</th>
                  <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase hidden md:table-cell">Size</th>
                  <th class="px-4 py-3 text-right text-xs font-medium text-muted-foreground uppercase">Actions</th>
                </tr>
              </thead>
              <tbody>
                {#each recentHistory as backup (backup.id)}
                  <tr class="border-t border-border hover:bg-muted/30 transition-colors">
                    <td class="px-4 py-3">
                      <p class="text-sm text-foreground">{timeAgo(backup.created_at)}</p>
                      <p class="text-xs text-muted-foreground">{formatDateTime(backup.created_at)}</p>
                    </td>
                    <td class="px-4 py-3 hidden sm:table-cell">
                      <p class="text-xs text-muted-foreground truncate max-w-[120px]">
                        {sites.find(s => s.id === backup.site_id)?.domain ?? backup.site_id.slice(0, 8) + '…'}
                      </p>
                    </td>
                    <td class="px-4 py-3">
                      <div class="flex items-center gap-1.5">
                        {#if backup.status === 'running'}
                          <svg class="w-3 h-3 animate-spin text-amber-400 shrink-0" fill="none" viewBox="0 0 24 24">
                            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
                          </svg>
                        {/if}
                        <span class="px-2 py-0.5 rounded-full text-xs font-medium {statusBadgeClass(backup.status)} capitalize">
                          {backup.status}
                        </span>
                      </div>
                      <span class="mt-1 inline-block bg-muted text-muted-foreground border border-border px-2 py-0.5 rounded-full text-xs">
                        {backup.type?.replace(/_/g, ' ') ?? 'full'}
                      </span>
                    </td>
                    <td class="px-4 py-3 text-xs text-muted-foreground hidden md:table-cell">
                      {formatBytes(backup.size_bytes)}
                    </td>
                    <td class="px-4 py-3">
                      <div class="flex items-center gap-1 justify-end">
                        {#if backup.status === 'completed'}
                          <button
                            class="h-8 px-3 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-xs font-medium hover:bg-red-500/20 inline-flex items-center gap-1"
                            on:click={() => openRestoreModal(backup)}
                          >
                            Restore
                          </button>
                          <a
                            href={api.backups.download(backup.id)}
                            class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1 transition-colors"
                          >
                            Download
                          </a>
                        {/if}
                        {#if backup.status === 'running'}
                          <div class="flex items-center gap-1.5 text-xs text-amber-400">
                            <span class="w-2 h-2 rounded-full bg-amber-400 animate-pulse"></span>
                            In progress
                          </div>
                        {/if}
                      </div>
                    </td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        </div>

      {:else}
        <!-- Timeline view -->
        <div class="relative pl-2">
          <!-- Vertical line -->
          <div class="absolute left-6 top-0 bottom-0 w-px bg-border"></div>

          {#each groupedByDate as group}
            <!-- Date header -->
            <div class="flex items-center gap-3 mb-3 mt-5 first:mt-0 relative z-10">
              <div class="w-8 h-8 rounded-full bg-card border-2 border-border flex items-center justify-center shrink-0">
                <svg class="w-3.5 h-3.5 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"/>
                </svg>
              </div>
              <span class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">{group.date}</span>
            </div>

            {#each group.backups as backup (backup.id)}
              <div class="ml-14 mb-3 bg-card border border-border rounded-xl p-4
                          transition-all duration-200 hover:-translate-y-0.5 hover:shadow-lg hover:shadow-black/20 hover:border-border/80">
                <div class="flex items-start justify-between gap-3">
                  <div class="space-y-1 min-w-0">
                    <div class="flex items-center gap-2 flex-wrap">
                      <span class="px-2 py-0.5 rounded-full text-xs font-medium {statusBadgeClass(backup.status)} capitalize">
                        {backup.status}
                      </span>
                      <span class="bg-muted text-muted-foreground border border-border px-2 py-0.5 rounded-full text-xs">
                        {backup.type?.replace(/_/g, ' ') ?? 'full'}
                      </span>
                      {#if backup.status === 'running'}
                        <svg class="w-3 h-3 animate-spin text-amber-400" fill="none" viewBox="0 0 24 24">
                          <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                          <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
                        </svg>
                      {/if}
                    </div>
                    <p class="text-xs text-muted-foreground truncate">
                      {sites.find(s => s.id === backup.site_id)?.domain ?? backup.site_id.slice(0, 8) + '…'}
                      {#if backup.size_bytes}
                        &middot; {formatBytes(backup.size_bytes)}
                      {/if}
                    </p>
                    <p class="text-xs text-muted-foreground/70">{formatDateTime(backup.created_at)}</p>
                  </div>
                  {#if backup.status === 'completed'}
                    <div class="flex items-center gap-1 shrink-0">
                      <button
                        class="h-7 px-2.5 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-xs font-medium hover:bg-red-500/20 transition-colors"
                        on:click={() => openRestoreModal(backup)}
                      >
                        Restore
                      </button>
                      <a
                        href={api.backups.download(backup.id)}
                        class="h-7 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center transition-colors"
                      >
                        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M12 10v6m0 0l-3-3m3 3l3-3m2 8H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>
                        </svg>
                      </a>
                    </div>
                  {/if}
                </div>
              </div>
            {/each}
          {/each}
        </div>
      {/if}
    </div>

  </div><!-- /two-col -->

</div><!-- /page -->

<!-- ── Add Schedule Modal ───────────────────────────────────────────────── -->
{#if showAddSchedule}
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showAddSchedule = false}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl space-y-4 fade-up">
      <div class="flex items-center justify-between">
        <h3 class="text-base font-bold text-foreground">Add Backup Schedule</h3>
        <button class="text-muted-foreground hover:text-foreground" on:click={() => showAddSchedule = false}>
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <div class="space-y-1">
        <label class="text-xs font-medium text-muted-foreground">Site</label>
        <select bind:value={sched_siteId}
                class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
          {#each sites as s}<option value={s.id}>{s.domain}</option>{/each}
        </select>
      </div>

      <div class="grid grid-cols-2 gap-4">
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Frequency</label>
          <select bind:value={sched_frequency}
                  class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
            <option value="hourly">Hourly</option>
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
            <option value="monthly">Monthly</option>
          </select>
        </div>
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Time (24h)</label>
          <input type="time" bind:value={sched_time}
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>
      </div>

      <div class="grid grid-cols-2 gap-4">
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Keep daily backups</label>
          <input type="number" bind:value={sched_retentionD} min="1" max="365"
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>
        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">Keep weekly backups</label>
          <input type="number" bind:value={sched_retentionW} min="1" max="52"
                 class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>
      </div>

      <div class="flex gap-2 pt-1">
        <button
          class="flex-1 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 disabled:opacity-50"
          on:click={saveSchedule}
          disabled={sched_saving || !sched_siteId}
        >
          {sched_saving ? 'Saving…' : 'Save Schedule'}
        </button>
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
          on:click={() => showAddSchedule = false}
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Restore 2-step Modal ─────────────────────────────────────────────── -->
{#if showRestoreModal && restoreTarget}
  {@const domain = sites.find(s => s.id === restoreTarget!.site_id)?.domain ?? ''}
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showRestoreModal = false}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl space-y-4 fade-up">

      <!-- Step indicator -->
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <div class="flex items-center gap-1.5">
            <div class="w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold
              {restoreStep === 1 ? 'bg-primary text-primary-foreground' : 'bg-green-500 text-white'}">
              {#if restoreStep === 2}
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="3" d="M5 13l4 4L19 7"/>
                </svg>
              {:else}
                1
              {/if}
            </div>
            <span class="text-xs {restoreStep === 1 ? 'text-foreground font-medium' : 'text-muted-foreground'}">Preview</span>
          </div>
          <div class="w-8 h-px bg-border"></div>
          <div class="flex items-center gap-1.5">
            <div class="w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold
              {restoreStep === 2 ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground'}">
              2
            </div>
            <span class="text-xs {restoreStep === 2 ? 'text-foreground font-medium' : 'text-muted-foreground'}">Confirm</span>
          </div>
        </div>
        <button class="text-muted-foreground hover:text-foreground" on:click={() => showRestoreModal = false}>
          <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      {#if restoreStep === 1}
        <!-- Step 1: Preview -->
        <div>
          <h3 class="text-base font-bold text-foreground mb-1">Restore Preview</h3>
          <p class="text-sm text-muted-foreground">Review what will be restored for <strong class="text-foreground">{domain}</strong>.</p>
        </div>

        <!-- Backup details -->
        <div class="bg-muted/30 border border-border rounded-xl p-4 space-y-2 text-sm">
          <div class="flex justify-between">
            <span class="text-muted-foreground">Date</span>
            <span class="text-foreground font-medium">{formatDateTime(restoreTarget.created_at)}</span>
          </div>
          <div class="flex justify-between">
            <span class="text-muted-foreground">Size</span>
            <span class="text-foreground font-medium">{formatBytes(restoreTarget.size_bytes)}</span>
          </div>
          <div class="flex justify-between">
            <span class="text-muted-foreground">Type</span>
            <span class="text-foreground font-medium capitalize">{restoreTarget.type?.replace(/_/g, ' ') ?? 'full'}</span>
          </div>
        </div>

        <!-- What will be restored -->
        <div class="space-y-2">
          <p class="text-xs font-semibold text-muted-foreground uppercase tracking-wide">What to restore</p>
          <label class="flex items-center gap-3 p-3 rounded-xl border border-border bg-muted/20 hover:bg-muted/40 cursor-pointer transition-colors">
            <input type="checkbox" bind:checked={restoreFiles} class="w-4 h-4 accent-primary rounded" />
            <div>
              <p class="text-sm font-medium text-foreground">Files</p>
              <p class="text-xs text-muted-foreground">All site files, themes, plugins, uploads</p>
            </div>
            {#if restoreFiles}
              <svg class="w-4 h-4 text-green-400 ml-auto shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
              </svg>
            {/if}
          </label>
          <label class="flex items-center gap-3 p-3 rounded-xl border border-border bg-muted/20 hover:bg-muted/40 cursor-pointer transition-colors">
            <input type="checkbox" bind:checked={restoreDatabase} class="w-4 h-4 accent-primary rounded" />
            <div>
              <p class="text-sm font-medium text-foreground">Database</p>
              <p class="text-xs text-muted-foreground">MySQL/MariaDB tables, posts, users, settings</p>
            </div>
            {#if restoreDatabase}
              <svg class="w-4 h-4 text-green-400 ml-auto shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
              </svg>
            {/if}
          </label>
        </div>

        <!-- Warning banner -->
        <div class="flex items-start gap-3 p-4 rounded-xl bg-red-500/10 border border-red-500/30">
          <svg class="w-5 h-5 text-red-400 shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
          </svg>
          <div>
            <p class="text-sm font-semibold text-red-400">This will overwrite ALL current files and database</p>
            <p class="text-xs text-red-400/80 mt-0.5">This action cannot be undone. Make sure you have a recent backup before proceeding.</p>
          </div>
        </div>

        <div class="flex gap-2 pt-1">
          <button
            class="flex-1 h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center justify-center gap-2 disabled:opacity-50"
            disabled={!restoreFiles && !restoreDatabase}
            on:click={() => restoreStep = 2}
          >
            Continue to Confirm
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
            </svg>
          </button>
          <button
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
            on:click={() => showRestoreModal = false}
          >
            Cancel
          </button>
        </div>

      {:else}
        <!-- Step 2: Confirm -->
        <div class="flex items-start gap-3">
          <div class="w-10 h-10 rounded-full bg-red-500/10 border border-red-500/20 flex items-center justify-center shrink-0">
            <svg class="w-5 h-5 text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732
                   4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
            </svg>
          </div>
          <div>
            <h3 class="text-base font-bold text-foreground">Confirm Restore</h3>
            <p class="text-sm text-muted-foreground mt-1">
              This will overwrite
              {#if restoreFiles && restoreDatabase}files and database{:else if restoreFiles}files only{:else}database only{/if}
              for <strong class="text-foreground">{domain}</strong>.
            </p>
          </div>
        </div>

        <div class="bg-red-500/5 border border-red-500/20 rounded-lg p-3 text-sm text-red-400">
          Backup from {formatDateTime(restoreTarget.created_at)} &middot; {formatBytes(restoreTarget.size_bytes)}
        </div>

        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground">
            Type <strong class="text-foreground">{domain}</strong> to confirm
          </label>
          <input
            bind:value={restoreConfirmDomain}
            placeholder={domain}
            class="h-9 w-full rounded-lg border border-red-500/30 bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-red-500/50 focus:border-red-500"
          />
        </div>

        <div class="flex gap-2">
          <button
            class="flex-1 h-9 px-4 rounded-lg bg-red-500 text-white text-sm font-medium hover:bg-red-600 inline-flex items-center justify-center gap-2 disabled:opacity-50 transition-colors"
            on:click={confirmRestore}
            disabled={restoreConfirmDomain !== domain || restoring}
          >
            {#if restoring}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
              </svg>
              Starting restore…
            {:else}
              Restore Backup
            {/if}
          </button>
          <button
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
            on:click={() => restoreStep = 1}
          >
            Back
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

<!-- ── Toast ────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
              flex items-center gap-2 slide-in-right"
       role="status" aria-live="polite">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 text-green-400 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}
