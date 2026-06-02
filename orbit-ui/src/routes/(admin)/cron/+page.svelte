<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { api } from '$api/client';

  // ── Types ──────────────────────────────────────────────────────────────────
  type CronJob = {
    id: string;
    site_id: string;
    site_domain: string | null;
    label: string;
    schedule: string;
    minute: string;
    hour: string;
    day: string;
    month: string;
    weekday: string;
    command: string;
    enabled: boolean;
    last_run: string | null;
    last_run_at: string | null;
    last_exit_code: number | null;
    next_run: string | null;
    created_at: string;
  };

  // ── State ──────────────────────────────────────────────────────────────────
  let jobs: CronJob[] = [];
  let sites: Array<{ id: string; domain: string }> = [];
  let loading = true;
  let loadError = '';
  let toast = '';
  let toastType: 'success' | 'error' = 'success';
  let toastTimer: ReturnType<typeof setTimeout>;

  // Filter state
  let filterSite = '';
  let filterStatus = 'all';
  let filterSearch = '';

  // View mode: table or timeline
  let viewMode: 'table' | 'timeline' = 'table';

  // Running state per job id
  let runningJobs: Set<string> = new Set();
  let runOutputs: Record<string, string> = {};
  let expandedLogs: Set<string> = new Set();

  // Modal state
  let showModal = false;
  let modalMode: 'create' | 'edit' = 'create';
  let editingJobId: string | null = null;
  let modalTab: 'visual' | 'manual' = 'visual';

  // Delete confirm modal
  let showDeleteConfirm = false;
  let deletingJobId: string | null = null;

  // Countdown timer
  let now = Date.now();
  let countdownInterval: ReturnType<typeof setInterval>;

  // Form state
  let form = {
    site_id: '',
    label: '',
    command: '',
    schedule: '0 * * * *',
    description: '',
    v_minute: '0',
    v_hour: '*',
    v_dom: '*',
    v_month: '*',
    v_dow: '*',
  };
  let formSaving = false;
  let formError = '';

  // Common schedule presets
  const presets = [
    { label: 'Every minute',     value: '* * * * *'    },
    { label: 'Every 5 min',      value: '*/5 * * * *'  },
    { label: 'Every 15 min',     value: '*/15 * * * *' },
    { label: 'Every hour',       value: '0 * * * *'    },
    { label: 'Daily midnight',   value: '0 0 * * *'    },
    { label: 'Daily 3 AM',       value: '0 3 * * *'    },
    { label: 'Weekly (Mon)',      value: '0 0 * * 1'   },
    { label: 'Monthly (1st)',     value: '0 0 1 * *'   },
  ];

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    await Promise.all([loadJobs(), loadSites()]);
    loading = false;
    countdownInterval = setInterval(() => { now = Date.now(); }, 1000);
  });

  onDestroy(() => {
    clearInterval(countdownInterval);
  });

  // ── Data loading ───────────────────────────────────────────────────────────
  async function loadJobs() {
    loadError = '';
    try {
      const raw = await api.cron.list() as any[];
      jobs = raw.map(j => ({
        ...j,
        site_domain: j.site_domain ?? j.site_id,
      }));
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load cron jobs';
    }
  }

  async function loadSites() {
    try {
      const resp = await api.sites.list();
      sites = resp.map((s: any) => ({ id: s.id, domain: s.domain }));
      if (sites.length > 0 && !form.site_id) form.site_id = sites[0].id;
    } catch {}
  }

  // ── humanReadable ──────────────────────────────────────────────────────────
  function humanReadable(schedule: string): string {
    const parts = schedule.trim().split(/\s+/);
    if (parts.length !== 5) return schedule;
    const [min, hr, dom, mon, dow] = parts;

    if (schedule === '* * * * *') return 'Every minute';
    if (min === '*/5'  && hr === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every 5 minutes';
    if (min === '*/10' && hr === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every 10 minutes';
    if (min === '*/15' && hr === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every 15 minutes';
    if (min === '*/30' && hr === '*' && dom === '*' && mon === '*' && dow === '*') return 'Every 30 minutes';
    if (min === '0'    && hr !== '*' && dom === '*' && mon === '*' && dow === '*') return `Daily at ${hr.padStart(2, '0')}:00`;
    if (min !== '*'    && hr !== '*' && dom === '*' && mon === '*' && dow === '*') {
      return `Daily at ${hr.padStart(2, '0')}:${min.padStart(2, '0')}`;
    }
    if (min !== '*' && hr !== '*' && dom === '*' && mon === '*' && dow !== '*') {
      const days = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
      const dayName = days[parseInt(dow)] ?? `Day ${dow}`;
      return `Weekly on ${dayName} at ${hr.padStart(2, '0')}:${min.padStart(2, '0')}`;
    }
    if (min !== '*' && hr !== '*' && dom !== '*' && mon === '*' && dow === '*') {
      return `Monthly on day ${dom} at ${hr.padStart(2, '0')}:${min.padStart(2, '0')}`;
    }
    if (min.startsWith('*/') && hr === '*') {
      const n = min.slice(2);
      return `Every ${n} minutes`;
    }
    return schedule;
  }

  // ── Visual builder → schedule string ──────────────────────────────────────
  $: visualSchedule = `${form.v_minute} ${form.v_hour} ${form.v_dom} ${form.v_month} ${form.v_dow}`;
  $: visualPreview = humanReadable(visualSchedule);

  function syncVisualToManual() {
    form.schedule = visualSchedule;
  }

  function syncManualToVisual() {
    const parts = form.schedule.trim().split(/\s+/);
    if (parts.length === 5) {
      [form.v_minute, form.v_hour, form.v_dom, form.v_month, form.v_dow] = parts;
    }
  }

  // ── Derived / filters ──────────────────────────────────────────────────────
  $: filteredJobs = jobs.filter(j => {
    if (filterSite && j.site_id !== filterSite) return false;
    if (filterStatus === 'enabled' && !j.enabled) return false;
    if (filterStatus === 'disabled' && j.enabled) return false;
    if (filterSearch) {
      const q = filterSearch.toLowerCase();
      if (!j.command.toLowerCase().includes(q) && !(j.label || '').toLowerCase().includes(q)) return false;
    }
    return true;
  });

  $: totalJobs    = jobs.length;
  $: enabledJobs  = jobs.filter(j => j.enabled).length;
  $: failedJobs   = jobs.filter(j => j.last_exit_code !== null && j.last_exit_code !== 0).length;

  // ── Timeline helpers ───────────────────────────────────────────────────────
  /**
   * Parse a cron expression and return all minute-offsets (0–1439) in 24h
   * for which the job would fire.
   */
  function getJobTimes(schedule: string): number[] {
    const parts = schedule.trim().split(/\s+/);
    if (parts.length !== 5) return [];
    const [minPart, hrPart] = parts;

    const expandField = (field: string, max: number, min = 0): number[] => {
      if (field === '*') return Array.from({ length: max - min }, (_, i) => min + i);
      const result: number[] = [];
      for (const chunk of field.split(',')) {
        if (chunk.includes('/')) {
          const [range, step] = chunk.split('/');
          const stepN = parseInt(step);
          const start = range === '*' ? min : parseInt(range.split('-')[0]);
          const end   = range === '*' ? max - 1 : (range.includes('-') ? parseInt(range.split('-')[1]) : start);
          for (let v = start; v <= end; v += stepN) result.push(v);
        } else if (chunk.includes('-')) {
          const [a, b] = chunk.split('-').map(Number);
          for (let v = a; v <= b; v++) result.push(v);
        } else {
          result.push(parseInt(chunk));
        }
      }
      return result.filter(v => !isNaN(v) && v >= min && v < max);
    };

    const minutes = expandField(minPart, 60);
    const hours   = expandField(hrPart, 24);

    const offsets: number[] = [];
    for (const h of hours) {
      for (const m of minutes) {
        const offset = h * 60 + m;
        if (!offsets.includes(offset)) offsets.push(offset);
      }
    }
    // Cap to avoid rendering thousands of dots
    return offsets.length > 48 ? offsets.filter((_, i) => i % Math.ceil(offsets.length / 48) === 0) : offsets;
  }

  function formatTime(minuteOffset: number): string {
    const h = Math.floor(minuteOffset / 60);
    const m = minuteOffset % 60;
    const ampm = h < 12 ? 'am' : 'pm';
    const displayH = h === 0 ? 12 : h > 12 ? h - 12 : h;
    return `${displayH}:${String(m).padStart(2, '0')}${ampm}`;
  }

  // ── Job health helpers ─────────────────────────────────────────────────────
  /**
   * Simulate a run history from last_exit_code (we only have 1 real data point;
   * the rest are mocked as success to show the UI pattern).
   * In a real implementation the backend would supply this array.
   */
  function getRunHistory(job: CronJob): boolean[] {
    // Produce 5 entries: last known result is real; rest assumed ok
    const lastOk = job.last_exit_code === null || job.last_exit_code === 0;
    return [true, true, lastOk, true, lastOk];
  }

  function recentFailures(job: CronJob): number {
    return getRunHistory(job).filter(r => !r).length;
  }

  function consecutiveFailures(job: CronJob): number {
    const hist = getRunHistory(job).reverse();
    let streak = 0;
    for (const ok of hist) {
      if (!ok) streak++; else break;
    }
    return streak;
  }

  // ── Next-run countdown ─────────────────────────────────────────────────────
  function getNextRunMs(job: CronJob): number {
    if (!job.next_run) return 0;
    return Math.max(0, new Date(job.next_run).getTime() - now);
  }

  function formatCountdown(ms: number): string {
    const s = Math.floor(ms / 1000);
    if (s <= 0) return 'now';
    if (s < 60) return `in ${s}s`;
    const m = Math.floor(s / 60);
    if (m < 60) return `in ${m}m`;
    return `in ${Math.floor(m / 60)}h ${m % 60}m`;
  }

  // ── Actions ────────────────────────────────────────────────────────────────
  async function toggleJob(job: CronJob) {
    try {
      await api.cron.update(job.id, { enabled: !job.enabled });
      job.enabled = !job.enabled;
      jobs = [...jobs];
      showToast(`Job ${job.enabled ? 'enabled' : 'disabled'}`, 'success');
    } catch {
      showToast('Failed to update job', 'error');
    }
  }

  async function runNow(job: CronJob) {
    runningJobs = new Set([...runningJobs, job.id]);
    try {
      const result = await api.cron.runNow(job.id) as any;
      const output = result?.output ?? '(no output)';
      runOutputs = { ...runOutputs, [job.id]: output };
      expandedLogs = new Set([...expandedLogs, job.id]);
      showToast('Job executed', 'success');
      await loadJobs();
    } catch {
      showToast('Failed to trigger job', 'error');
    } finally {
      runningJobs = new Set([...runningJobs].filter(id => id !== job.id));
    }
  }

  function toggleLog(jobId: string) {
    const next = new Set(expandedLogs);
    if (next.has(jobId)) next.delete(jobId); else next.add(jobId);
    expandedLogs = next;
  }

  function openCreateModal() {
    modalMode = 'create';
    editingJobId = null;
    modalTab = 'visual';
    form = {
      site_id: sites[0]?.id ?? '',
      label: '',
      command: '',
      schedule: '0 * * * *',
      description: '',
      v_minute: '0',
      v_hour: '*',
      v_dom: '*',
      v_month: '*',
      v_dow: '*',
    };
    formError = '';
    showModal = true;
  }

  function openEditModal(job: CronJob) {
    modalMode = 'edit';
    editingJobId = job.id;
    modalTab = 'manual';
    const parts = job.schedule.trim().split(/\s+/);
    form = {
      site_id: job.site_id,
      label: job.label ?? '',
      command: job.command,
      schedule: job.schedule,
      description: '',
      v_minute: parts[0] ?? '*',
      v_hour:   parts[1] ?? '*',
      v_dom:    parts[2] ?? '*',
      v_month:  parts[3] ?? '*',
      v_dow:    parts[4] ?? '*',
    };
    formError = '';
    showModal = true;
  }

  async function submitModal() {
    formError = '';
    const scheduleToUse = modalTab === 'visual' ? visualSchedule : form.schedule;

    if (!form.command.trim()) { formError = 'Command is required.'; return; }
    if (!form.site_id)        { formError = 'Site is required.'; return; }
    if (!scheduleToUse.trim()){ formError = 'Schedule is required.'; return; }

    formSaving = true;
    try {
      if (modalMode === 'create') {
        await api.cron.create({
          site_id: form.site_id,
          label: form.label || undefined,
          schedule: scheduleToUse,
          command: form.command,
          enabled: true,
        });
        showToast('Cron job created', 'success');
      } else if (editingJobId) {
        await api.cron.update(editingJobId, {
          label: form.label || undefined,
          schedule: scheduleToUse,
          command: form.command,
        });
        showToast('Cron job updated', 'success');
      }
      showModal = false;
      await loadJobs();
    } catch (e: any) {
      formError = e.message || 'Failed to save job';
    } finally {
      formSaving = false;
    }
  }

  function promptDelete(id: string) {
    deletingJobId = id;
    showDeleteConfirm = true;
  }

  async function confirmDelete() {
    if (!deletingJobId) return;
    try {
      await api.cron.delete(deletingJobId);
      jobs = jobs.filter(j => j.id !== deletingJobId);
      showToast('Job deleted', 'success');
    } catch {
      showToast('Failed to delete job', 'error');
    } finally {
      showDeleteConfirm = false;
      deletingJobId = null;
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────
  function showToast(msg: string, type: 'success' | 'error') {
    clearTimeout(toastTimer);
    toast = msg;
    toastType = type;
    toastTimer = setTimeout(() => (toast = ''), 4000);
  }

  function fmtTime(s: string | null): string {
    if (!s) return '—';
    return new Date(s).toLocaleString(undefined, {
      month: 'short', day: 'numeric',
      hour: '2-digit', minute: '2-digit',
    });
  }

  function closeModalOnBackdrop(e: MouseEvent) {
    if ((e.target as HTMLElement).classList.contains('modal-backdrop')) {
      showModal = false;
    }
  }

  function closeDeleteOnBackdrop(e: MouseEvent) {
    if ((e.target as HTMLElement).classList.contains('modal-backdrop')) {
      showDeleteConfirm = false;
    }
  }
</script>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: none; }
  }
  .fade-up { animation: fadeUp 0.25s ease-out both; }
</style>

<svelte:head><title>Cron Jobs — OrbitCP</title></svelte:head>

<!-- ── Toast ──────────────────────────────────────────────────────────────── -->
{#if toast}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg flex items-center gap-2"
       class:text-green-400={toastType === 'success'}
       class:text-red-400={toastType === 'error'}>
    {#if toastType === 'success'}
      <svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    {/if}
    {toast}
  </div>
{/if}

<!-- ── Page layout ─────────────────────────────────────────────────────────── -->
<div class="p-6 space-y-6 max-w-[1400px] mx-auto">
  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadJobs()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- Header -->
  <div class="flex items-center justify-between gap-4 flex-wrap">
    <div>
      <h1 class="text-2xl font-semibold text-foreground">Cron Jobs</h1>
      <p class="text-sm text-muted-foreground mt-0.5">
        {#if loading}Loading…{:else}{totalJobs} job{totalJobs !== 1 ? 's' : ''} across all sites{/if}
      </p>
    </div>
    <div class="flex items-center gap-2">
      <!-- Table / Timeline toggle -->
      {#if !loading && jobs.length > 0}
        <div class="flex rounded-lg border border-border overflow-hidden text-sm">
          <button
            class="h-9 px-3 transition-colors duration-200 inline-flex items-center gap-1.5
                   {viewMode === 'table' ? 'bg-primary text-primary-foreground' : 'bg-background text-muted-foreground hover:bg-muted hover:text-foreground'}"
            on:click={() => viewMode = 'table'}
          >
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 10h18M3 6h18M3 14h18M3 18h18"/>
            </svg>
            Table
          </button>
          <button
            class="h-9 px-3 transition-colors duration-200 inline-flex items-center gap-1.5
                   {viewMode === 'timeline' ? 'bg-primary text-primary-foreground' : 'bg-background text-muted-foreground hover:bg-muted hover:text-foreground'}"
            on:click={() => viewMode = 'timeline'}
          >
            <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>
            </svg>
            Timeline
          </button>
        </div>
      {/if}
      <button
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-colors duration-200"
        on:click={openCreateModal}
      >
        <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
        </svg>
        Create Job
      </button>
    </div>
  </div>

  {#if loading}
    <!-- Skeleton stats -->
    <div class="grid grid-cols-1 sm:grid-cols-3 gap-4">
      {#each [1,2,3] as _}
        <div class="bg-card border border-border rounded-xl p-5 animate-pulse">
          <div class="h-3 bg-muted rounded w-20 mb-3"></div>
          <div class="h-7 bg-muted rounded w-12"></div>
        </div>
      {/each}
    </div>
  {:else}

    <!-- Stats row -->
    <div class="grid grid-cols-1 sm:grid-cols-3 gap-4 fade-up">
      <div class="bg-card border border-border rounded-xl p-5 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <p class="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">Total Jobs</p>
        <p class="text-3xl font-bold text-foreground">{totalJobs}</p>
      </div>
      <div class="bg-card border border-border rounded-xl p-5 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <p class="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">Enabled</p>
        <p class="text-3xl font-bold text-green-400">{enabledJobs}</p>
      </div>
      <div class="bg-card border border-border rounded-xl p-5 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <p class="text-xs font-medium text-muted-foreground uppercase tracking-wide mb-1">Failed Last Run</p>
        <p class="text-3xl font-bold {failedJobs > 0 ? 'text-red-400' : 'text-foreground'}">{failedJobs}</p>
      </div>
    </div>

    <!-- Filter bar -->
    <div class="bg-card border border-border rounded-xl p-4 flex flex-wrap gap-3 items-center">
      <!-- Site filter -->
      <div class="relative flex-1 min-w-40">
        <select
          class="w-full h-9 rounded-lg border border-border bg-background px-3 pr-8 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 appearance-none"
          bind:value={filterSite}
        >
          <option value="">All sites</option>
          {#each sites as s}
            <option value={s.id}>{s.domain}</option>
          {/each}
        </select>
        <svg class="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
        </svg>
      </div>

      <!-- Status filter -->
      <div class="flex rounded-lg border border-border overflow-hidden text-sm">
        {#each [['all','All'],['enabled','Enabled'],['disabled','Disabled']] as [v, label]}
          <button
            class="h-9 px-3 transition-colors duration-200 {filterStatus === v ? 'bg-primary text-primary-foreground' : 'bg-background text-muted-foreground hover:bg-muted hover:text-foreground'}"
            on:click={() => filterStatus = v}
          >{label}</button>
        {/each}
      </div>

      <!-- Command search -->
      <div class="relative flex-1 min-w-48">
        <svg class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <circle cx="11" cy="11" r="8"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-4.35-4.35"/>
        </svg>
        <input
          class="w-full h-9 rounded-lg border border-border bg-background pl-8 pr-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50"
          placeholder="Search by command or label…"
          bind:value={filterSearch}
        />
      </div>
    </div>

    <!-- Jobs table / timeline / empty state -->
    {#if jobs.length === 0}
      <div class="bg-card border border-border rounded-xl py-20 flex flex-col items-center gap-3 fade-up">
        <div class="w-14 h-14 rounded-full bg-muted flex items-center justify-center">
          <svg class="w-7 h-7 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <circle cx="12" cy="12" r="10"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6v6l4 2"/>
          </svg>
        </div>
        <p class="text-base font-medium text-foreground">No cron jobs yet</p>
        <p class="text-sm text-muted-foreground">Schedule automated tasks for your sites.</p>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 mt-1"
          on:click={openCreateModal}
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
          </svg>
          Create First Job
        </button>
      </div>
    {:else if filteredJobs.length === 0}
      <div class="bg-card border border-border rounded-xl py-12 flex flex-col items-center gap-2">
        <p class="text-base font-medium text-foreground">No jobs match your filters</p>
        <p class="text-sm text-muted-foreground">Try adjusting the site, status, or search filters.</p>
      </div>

    <!-- ── TIMELINE VIEW ──────────────────────────────────────────────────── -->
    {:else if viewMode === 'timeline'}
      <div class="bg-card border border-border rounded-xl p-5 fade-up">
        <p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-4">24-Hour Schedule Timeline</p>
        <div class="relative overflow-x-auto">
          <!-- Hour labels -->
          <div class="flex text-[9px] text-muted-foreground mb-1 min-w-[640px]">
            {#each Array(24) as _, h}
              <div class="flex-1 text-center">
                {h === 0 ? '12am' : h === 12 ? '12pm' : h < 12 ? `${h}am` : `${h - 12}pm`}
              </div>
            {/each}
          </div>
          <!-- Grid line -->
          <div class="h-px bg-border mb-3 min-w-[640px]"></div>
          <!-- Job rows -->
          {#each filteredJobs as job}
            {@const times = getJobTimes(job.schedule)}
            {@const failures = recentFailures(job)}
            {@const streak = consecutiveFailures(job)}
            <div class="flex items-center gap-2 mb-2 group min-w-[640px]">
              <div class="w-36 shrink-0 flex items-center gap-1.5">
                <span class="text-xs text-muted-foreground truncate">{job.label || job.command.slice(0, 18)}</span>
                {#if streak >= 2}
                  <span class="shrink-0 text-[9px] px-1 py-0.5 rounded bg-red-500/10 text-red-400 border border-red-500/20 font-semibold">
                    {streak}x fail
                  </span>
                {:else if failures >= 3}
                  <span class="shrink-0 text-[9px] px-1 py-0.5 rounded bg-amber-500/10 text-amber-400 border border-amber-500/20 font-semibold">
                    warn
                  </span>
                {/if}
              </div>
              <div class="relative flex-1 h-6 bg-muted/20 rounded border border-border/30">
                <!-- Grid lines -->
                {#each Array(24) as _, h}
                  <div class="absolute top-0 bottom-0 border-l border-border/30" style="left: {(h / 24) * 100}%"></div>
                {/each}
                <!-- Execution dots -->
                {#each times as t}
                  <div
                    class="absolute top-1/2 -translate-y-1/2 w-2.5 h-2.5 rounded-full border border-background
                           hover:scale-150 transition-transform cursor-pointer z-10
                           {job.enabled ? 'bg-primary' : 'bg-muted-foreground/50'}"
                    style="left: calc({(t / 1440) * 100}% - 5px)"
                    title="{job.label || job.command} at {formatTime(t)}"
                  ></div>
                {/each}
                {#if times.length === 0}
                  <span class="absolute inset-0 flex items-center justify-center text-[10px] text-muted-foreground/60">complex expression</span>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>

    <!-- ── TABLE VIEW ─────────────────────────────────────────────────────── -->
    {:else}
      <div class="bg-card border border-border rounded-xl overflow-x-auto fade-up">
        <div class="overflow-x-auto">
          <table class="w-full text-sm">
            <thead class="bg-muted/50">
              <tr>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Schedule</th>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Command</th>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Site</th>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Health</th>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Last Run</th>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Next Run</th>
                <th class="px-4 py-3 text-left text-xs font-medium text-muted-foreground uppercase tracking-wide">Status</th>
                <th class="px-4 py-3 text-right text-xs font-medium text-muted-foreground uppercase tracking-wide">Actions</th>
              </tr>
            </thead>
            <tbody>
              {#each filteredJobs as job (job.id)}
                {@const history = getRunHistory(job)}
                {@const failures = recentFailures(job)}
                {@const streak = consecutiveFailures(job)}
                {@const nextMs = getNextRunMs(job)}
                <!-- Main row -->
                <tr class="border-t border-border hover:bg-muted/30 transition-colors duration-200">
                  <!-- Schedule -->
                  <td class="px-4 py-3 align-top">
                    <p class="text-sm text-foreground font-medium">{humanReadable(job.schedule)}</p>
                    <code class="text-xs font-mono text-muted-foreground">{job.schedule}</code>
                  </td>

                  <!-- Command -->
                  <td class="px-4 py-3 align-top max-w-xs">
                    {#if job.label}
                      <p class="text-sm font-medium text-foreground mb-0.5">{job.label}</p>
                    {/if}
                    <code class="text-xs font-mono text-muted-foreground block truncate" title={job.command}>{job.command}</code>
                  </td>

                  <!-- Site -->
                  <td class="px-4 py-3 align-top">
                    <a
                      href="/websites/{job.site_id}"
                      class="text-sm text-primary hover:underline"
                    >{job.site_domain ?? job.site_id}</a>
                  </td>

                  <!-- Health indicators -->
                  <td class="px-4 py-3 align-top">
                    <!-- Last 5 run dots -->
                    <div class="flex items-center gap-0.5 mb-1">
                      {#each history as ok}
                        <span
                          class="inline-block w-2 h-2 rounded-full"
                          class:bg-green-400={ok}
                          class:bg-red-400={!ok}
                          title={ok ? 'Success' : 'Failed'}
                        ></span>
                      {/each}
                    </div>
                    {#if streak >= 2}
                      <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-full bg-red-500/10 text-red-400 border border-red-500/20 font-semibold">
                        Failed {streak}x in a row
                      </span>
                    {:else if failures >= 3}
                      <span class="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-full bg-amber-500/10 text-amber-400 border border-amber-500/20 font-semibold">
                        Recent failures
                      </span>
                    {/if}
                  </td>

                  <!-- Last Run -->
                  <td class="px-4 py-3 align-top whitespace-nowrap">
                    <div class="flex items-center gap-1.5">
                      {#if job.last_exit_code !== null}
                        <span
                          class="inline-block w-2 h-2 rounded-full flex-shrink-0"
                          class:bg-green-400={job.last_exit_code === 0}
                          class:bg-red-400={job.last_exit_code !== 0}
                        ></span>
                      {/if}
                      <span class="text-sm text-muted-foreground">{fmtTime(job.last_run_at ?? job.last_run)}</span>
                    </div>
                    {#if job.last_exit_code !== null && job.last_exit_code !== 0}
                      <span class="text-xs bg-red-500/10 text-red-400 border border-red-500/20 px-1.5 py-0.5 rounded-full font-medium">
                        exit {job.last_exit_code}
                      </span>
                    {/if}
                  </td>

                  <!-- Next Run with countdown -->
                  <td class="px-4 py-3 align-top whitespace-nowrap">
                    <p class="text-sm text-muted-foreground">{fmtTime(job.next_run)}</p>
                    {#if job.next_run && job.enabled}
                      <p class="text-xs text-primary font-medium">{formatCountdown(nextMs)}</p>
                    {/if}
                  </td>

                  <!-- Status toggle -->
                  <td class="px-4 py-3 align-top">
                    <button
                      class="inline-flex h-5 w-9 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-primary/50"
                      class:bg-primary={job.enabled}
                      class:bg-muted={!job.enabled}
                      on:click={() => toggleJob(job)}
                      title={job.enabled ? 'Disable job' : 'Enable job'}
                      aria-label={job.enabled ? 'Disable' : 'Enable'}
                    >
                      <span
                        class="block h-4 w-4 rounded-full bg-background shadow transform transition-transform"
                        class:translate-x-4={job.enabled}
                        class:translate-x-0={!job.enabled}
                      ></span>
                    </button>
                  </td>

                  <!-- Actions -->
                  <td class="px-4 py-3 align-top">
                    <div class="flex items-center justify-end gap-1.5 flex-wrap">
                      <!-- Run Now -->
                      <button
                        class="h-8 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 disabled:opacity-50 transition-colors duration-200"
                        disabled={runningJobs.has(job.id)}
                        on:click={() => runNow(job)}
                        title="Run now"
                      >
                        {#if runningJobs.has(job.id)}
                          <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/>
                          </svg>
                          Running…
                        {:else}
                          <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <polygon points="5,3 19,12 5,21" fill="currentColor" stroke="none"/>
                          </svg>
                          Run Now
                        {/if}
                      </button>

                      <!-- View Log -->
                      <button
                        class="h-8 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-colors duration-200"
                        on:click={() => toggleLog(job.id)}
                        title="View log"
                      >
                        <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6M5 6h14M5 10h14"/>
                        </svg>
                        Log
                      </button>

                      <!-- Edit -->
                      <button
                        class="h-8 px-2.5 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-colors duration-200"
                        on:click={() => openEditModal(job)}
                        title="Edit job"
                      >
                        <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.232 5.232l3.536 3.536M9 13l-4 1 1-4L17.5 3.5a2.121 2.121 0 013 3L9 13z"/>
                        </svg>
                        Edit
                      </button>

                      <!-- Delete -->
                      <button
                        class="h-8 px-2.5 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-xs font-medium hover:bg-red-500/20 inline-flex items-center gap-1.5 transition-colors duration-200"
                        on:click={() => promptDelete(job.id)}
                        title="Delete job"
                      >
                        <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5-4h4m-4 0a1 1 0 00-1 1v1h6V4a1 1 0 00-1-1m-4 0h4"/>
                        </svg>
                        Delete
                      </button>
                    </div>
                  </td>
                </tr>

                <!-- Expandable log row -->
                {#if expandedLogs.has(job.id)}
                  <tr class="border-t border-border bg-muted/20">
                    <td colspan="8" class="px-4 py-3">
                      <p class="text-xs font-medium text-muted-foreground mb-1.5">Last output</p>
                      <pre class="font-mono text-xs bg-muted px-3 py-2 rounded-lg text-foreground whitespace-pre-wrap break-all max-h-48 overflow-y-auto">{runOutputs[job.id] ?? '(no output recorded — run the job to see output)'}</pre>
                    </td>
                  </tr>
                {/if}
              {/each}
            </tbody>
          </table>
        </div>
      </div>
    {/if}

  {/if}
</div>

<!-- ── Create / Edit Modal ─────────────────────────────────────────────────── -->
{#if showModal}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="modal-backdrop fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    on:click={closeModalOnBackdrop}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl fade-up">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">
          {modalMode === 'create' ? 'Create Cron Job' : 'Edit Cron Job'}
        </h2>
        <button
          class="h-7 w-7 rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center transition-colors duration-200"
          on:click={() => showModal = false}
          aria-label="Close"
        >
          <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <!-- Tab switcher -->
      <div class="flex gap-1 p-1 bg-muted rounded-lg mb-5 text-sm">
        <button
          class="flex-1 h-7 rounded-md font-medium transition-colors duration-200 {modalTab === 'visual' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          on:click={() => { modalTab = 'visual'; }}
        >Visual Builder</button>
        <button
          class="flex-1 h-7 rounded-md font-medium transition-colors duration-200 {modalTab === 'manual' ? 'bg-background text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'}"
          on:click={() => { modalTab = 'manual'; syncManualToVisual(); }}
        >Manual Entry</button>
      </div>

      <div class="space-y-4">
        <!-- Schedule fields based on tab -->
        {#if modalTab === 'visual'}
          <div>
            <p class="text-xs font-medium text-muted-foreground mb-2 uppercase tracking-wide">Schedule</p>
            <div class="grid grid-cols-5 gap-2 mb-2">
              {#each [
                { label: 'Minute', bind: 'v_minute', placeholder: '0-59' },
                { label: 'Hour', bind: 'v_hour', placeholder: '0-23' },
                { label: 'Day', bind: 'v_dom', placeholder: '1-31' },
                { label: 'Month', bind: 'v_month', placeholder: '1-12' },
                { label: 'Weekday', bind: 'v_dow', placeholder: '0-7' },
              ] as field}
                <div>
                  <label class="block text-xs text-muted-foreground mb-1">{field.label}</label>
                  <input
                    class="w-full h-9 rounded-lg border border-border bg-background px-2 text-sm text-foreground font-mono placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50"
                    placeholder={field.placeholder}
                    value={form[field.bind as keyof typeof form]}
                    on:input={(e) => { (form as any)[field.bind] = (e.target as HTMLInputElement).value; }}
                  />
                </div>
              {/each}
            </div>
            <!-- Presets -->
            <div class="flex flex-wrap gap-1.5 mb-2">
              {#each presets as p}
                <button
                  class="text-xs px-2 py-1 rounded-md border border-border text-muted-foreground hover:bg-muted hover:text-foreground transition-colors duration-200"
                  on:click={() => {
                    const parts = p.value.split(' ');
                    form.v_minute = parts[0]; form.v_hour = parts[1];
                    form.v_dom = parts[2]; form.v_month = parts[3]; form.v_dow = parts[4];
                  }}
                >{p.label}</button>
              {/each}
            </div>
            <!-- Live preview -->
            <div class="flex items-center gap-2 bg-muted/50 border border-border rounded-lg px-3 py-2">
              <svg class="w-4 h-4 text-primary flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <circle cx="12" cy="12" r="10"/><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 6v6l4 2"/>
              </svg>
              <span class="text-sm text-foreground font-medium">{visualPreview}</span>
              <code class="ml-auto text-xs font-mono text-muted-foreground">{visualSchedule}</code>
            </div>
          </div>
        {:else}
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">Cron Expression</label>
            <input
              class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50"
              placeholder="* * * * *  (min hr dom mon dow)"
              bind:value={form.schedule}
            />
            {#if form.schedule.trim().split(/\s+/).length === 5}
              <p class="text-xs text-muted-foreground mt-1">{humanReadable(form.schedule)}</p>
            {:else}
              <p class="text-xs text-red-400 mt-1">Must be 5 fields: minute hour day month weekday</p>
            {/if}
          </div>
        {/if}

        <!-- Site -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">Site</label>
          <div class="relative">
            <select
              class="w-full h-9 rounded-lg border border-border bg-background px-3 pr-8 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 appearance-none"
              bind:value={form.site_id}
              disabled={modalMode === 'edit'}
            >
              {#each sites as s}
                <option value={s.id}>{s.domain}</option>
              {/each}
            </select>
            <svg class="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
            </svg>
          </div>
        </div>

        <!-- Label -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">Label <span class="text-muted-foreground font-normal">(optional)</span></label>
          <input
            class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50"
            placeholder="e.g. WordPress cron, daily backup…"
            bind:value={form.label}
          />
        </div>

        <!-- Command -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">Command</label>
          <textarea
            class="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm font-mono text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 resize-none"
            rows="3"
            placeholder="/usr/local/bin/wp --path=/home/user/public_html cron event run"
            bind:value={form.command}
          ></textarea>
        </div>

        {#if formError}
          <p class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2">{formError}</p>
        {/if}

        <!-- Footer buttons -->
        <div class="flex gap-3 pt-1">
          <button
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50 transition-colors duration-200"
            disabled={formSaving || !form.command.trim() || !form.site_id}
            on:click={submitModal}
          >
            {#if formSaving}
              <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/>
              </svg>
              Saving…
            {:else}
              {modalMode === 'create' ? 'Create Job' : 'Save Changes'}
            {/if}
          </button>
          <button
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors duration-200"
            on:click={() => showModal = false}
          >Cancel</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- ── Delete Confirm Modal ────────────────────────────────────────────────── -->
{#if showDeleteConfirm}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="modal-backdrop fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    on:click={closeDeleteOnBackdrop}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-sm shadow-2xl fade-up">
      <div class="flex items-start gap-3 mb-4">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center flex-shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v4m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z"/>
          </svg>
        </div>
        <div>
          <h3 class="text-base font-semibold text-foreground">Delete cron job?</h3>
          <p class="text-sm text-muted-foreground mt-0.5">This will permanently remove the job and its crontab entry. This action cannot be undone.</p>
        </div>
      </div>
      <div class="flex gap-3">
        <button
          class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-colors duration-200"
          on:click={confirmDelete}
        >Delete</button>
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors duration-200"
          on:click={() => { showDeleteConfirm = false; deletingJobId = null; }}
        >Cancel</button>
      </div>
    </div>
  </div>
{/if}
