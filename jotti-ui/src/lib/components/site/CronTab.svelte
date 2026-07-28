<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import type { Site, CronJob } from '$api/client';
  import { auth } from '$lib/stores/auth';

  // ── Props ─────────────────────────────────────────────────────────────────────

  export let siteId: string;
  export let site: Site;

  // ── Auth helper ───────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const t = get(auth).token;
    return { 'Content-Type': 'application/json', ...(t ? { Authorization: 'Bearer ' + t } : {}) };
  }

  // ── State ─────────────────────────────────────────────────────────────────────

  let jobs: CronJob[] = [];
  let loading = true;

  // output panel: jobId → string
  let outputMap: Record<string, string> = {};
  let expandedOutputId: string | null = null;
  let runningJobId: string | null = null;

  // modal
  let modalOpen = false;
  let editingJob: CronJob | null = null;
  let deleteConfirmId: string | null = null;
  let saving = false;
  let deleting = false;

  // form state
  let formLabel = '';
  let formCommand = '';
  let formEnabled = true;
  let builderMode = true; // true = visual builder, false = raw crontab
  let fMin = '*'; let fHr = '*'; let fDom = '*'; let fMon = '*'; let fDow = '*';
  let rawSchedule = '* * * * *';

  // toast
  interface Toast { message: string; type: 'success' | 'error' }
  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function showToast(message: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 4000);
  }

  function formatDate(iso: string | null): string {
    if (!iso) return 'Never';
    return new Date(iso).toLocaleString('en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  }

  function humanReadable(schedule: string): string {
    const parts = schedule.trim().split(/\s+/);
    if (parts.length !== 5) return schedule;
    const [min, hr, dom, mon, dow] = parts;
    if (schedule === '* * * * *') return 'Every minute';
    if (schedule === '*/5 * * * *') return 'Every 5 minutes';
    if (schedule === '*/15 * * * *') return 'Every 15 minutes';
    if (schedule === '0 * * * *') return 'Every hour';
    if (min !== '*' && hr !== '*' && dom === '*' && mon === '*' && dow === '*')
      return `Daily at ${hr.padStart(2, '0')}:${min.padStart(2, '0')}`;
    if (min !== '*' && hr !== '*' && dow !== '*' && dom === '*' && mon === '*') {
      const days = ['Sun','Mon','Tue','Wed','Thu','Fri','Sat'];
      return `Weekly on ${days[parseInt(dow)] ?? 'day '+dow} at ${hr.padStart(2,'0')}:${min.padStart(2,'0')}`;
    }
    return schedule;
  }

  function computeSchedule(): string {
    return builderMode ? `${fMin} ${fHr} ${fDom} ${fMon} ${fDow}` : rawSchedule;
  }

  function syncBuilderFromRaw() {
    const parts = rawSchedule.trim().split(/\s+/);
    if (parts.length === 5) {
      [fMin, fHr, fDom, fMon, fDow] = parts;
    }
  }

  function syncRawFromBuilder() {
    rawSchedule = computeSchedule();
  }

  function applyPreset(sched: string) {
    rawSchedule = sched;
    builderMode = false;
  }

  // ── Data loading ──────────────────────────────────────────────────────────────

  onMount(loadJobs);
  onDestroy(() => { if (toastTimer) clearTimeout(toastTimer); });

  async function loadJobs() {
    loading = true;
    try {
      const res = await fetch(`/api/v1/cron/site/${siteId}`, { headers: authH() });
      if (res.ok) {
        jobs = await res.json();
      } else {
        jobs = [];
      }
    } catch {
      jobs = [];
    } finally {
      loading = false;
    }
  }

  // ── Modal helpers ─────────────────────────────────────────────────────────────

  function openCreate() {
    editingJob = null;
    formLabel = '';
    formCommand = `php /var/www/vhosts/${site.domain}/artisan schedule:run`;
    formEnabled = true;
    fMin = '*'; fHr = '*'; fDom = '*'; fMon = '*'; fDow = '*';
    rawSchedule = '* * * * *';
    builderMode = true;
    modalOpen = true;
  }

  function openEdit(job: CronJob) {
    editingJob = job;
    formLabel = job.label;
    formCommand = job.command;
    formEnabled = job.enabled;
    rawSchedule = job.schedule;
    syncBuilderFromRaw();
    builderMode = false;
    modalOpen = true;
  }

  function closeModal() {
    modalOpen = false;
    editingJob = null;
    saving = false;
  }

  // ── Actions ───────────────────────────────────────────────────────────────────

  async function saveJob() {
    saving = true;
    const schedule = computeSchedule();
    try {
      if (editingJob) {
        const res = await fetch(`/api/v1/cron/${editingJob.id}`, {
          method: 'PUT',
          headers: authH(),
          body: JSON.stringify({ label: formLabel, schedule, command: formCommand, enabled: formEnabled }),
        });
        if (!res.ok) {
          const err = await res.json().catch(() => ({} as { message?: string }));
          throw new Error((err as { message?: string }).message ?? `HTTP ${res.status}`);
        }
        showToast('Cron job updated', 'success');
      } else {
        const res = await fetch('/api/v1/cron', {
          method: 'POST',
          headers: authH(),
          body: JSON.stringify({ site_id: siteId, label: formLabel, schedule, command: formCommand, enabled: formEnabled }),
        });
        if (!res.ok) {
          const err = await res.json().catch(() => ({} as { message?: string }));
          throw new Error((err as { message?: string }).message ?? `HTTP ${res.status}`);
        }
        showToast('Cron job created', 'success');
      }
      closeModal();
      await loadJobs();
    } catch (err: unknown) {
      const fallbackMsg = editingJob ? 'Failed to update cron job' : 'Failed to create cron job';
      showToast(err instanceof Error ? err.message : fallbackMsg, 'error');
    } finally {
      saving = false;
    }
  }

  async function toggleJob(job: CronJob) {
    try {
      const res = await fetch(`/api/v1/cron/${job.id}`, {
        method: 'PUT',
        headers: authH(),
        body: JSON.stringify({ enabled: !job.enabled }),
      });
      if (!res.ok) throw new Error();
      jobs = jobs.map(j => j.id === job.id ? { ...j, enabled: !j.enabled } : j);
    } catch {
      showToast('Failed to toggle cron job', 'error');
    }
  }

  async function runJob(job: CronJob) {
    runningJobId = job.id;
    try {
      const res = await fetch(`/api/v1/cron/${job.id}/run`, { method: 'POST', headers: authH() });
      if (!res.ok) throw new Error();
      const data = await res.json().catch(() => ({}));
      outputMap = { ...outputMap, [job.id]: data.output ?? '(Job triggered — no output returned)' };
      expandedOutputId = job.id;
      showToast('Job triggered successfully', 'success');
      await loadJobs();
    } catch {
      showToast('Failed to run cron job', 'error');
    } finally {
      runningJobId = null;
    }
  }

  async function deleteJob(jobId: string) {
    deleting = true;
    try {
      const res = await fetch(`/api/v1/cron/${jobId}`, { method: 'DELETE', headers: authH() });
      if (!res.ok) throw new Error();
      jobs = jobs.filter(j => j.id !== jobId);
      deleteConfirmId = null;
      showToast('Cron job deleted', 'success');
    } catch {
      showToast('Failed to delete cron job', 'error');
    } finally {
      deleting = false;
    }
  }

  function toggleOutput(jobId: string) {
    expandedOutputId = expandedOutputId === jobId ? null : jobId;
  }
</script>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  .card-hover { transition: all 0.2s ease; }
  .card-hover:hover { transform: translateY(-1px); box-shadow: 0 8px 30px rgba(0,0,0,0.2); }
</style>

<!-- ── Header ─────────────────────────────────────────────────────────────────── -->
<div class="tab-content space-y-6">
  <div class="flex items-center justify-between gap-4">
    <div class="flex items-center gap-2.5">
      <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
          <path stroke-linecap="round" stroke-linejoin="round" d="M12 6v6h4.5m4.5 0a9 9 0 11-18 0 9 9 0 0118 0z"/>
        </svg>
      </div>
      <h2 class="text-sm font-semibold text-foreground">Cron Jobs</h2>
      {#if !loading}
        <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-muted text-muted-foreground border border-border">
          {jobs.length}
        </span>
      {/if}
    </div>
    <button
      on:click={openCreate}
      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors inline-flex items-center gap-2"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4" />
      </svg>
      New Cron Job
    </button>
  </div>

  <!-- ── Skeleton loader ────────────────────────────────────────────────────── -->
  {#if loading}
    <div class="space-y-3">
      {#each [0, 1, 2] as _}
        <div class="bg-card border border-border rounded-xl p-4 space-y-3">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-3">
              <div class="animate-pulse bg-muted rounded h-4 w-32"></div>
              <div class="animate-pulse bg-muted rounded h-5 w-24"></div>
            </div>
            <div class="animate-pulse bg-muted rounded h-6 w-10"></div>
          </div>
          <div class="animate-pulse bg-muted rounded h-3 w-64"></div>
          <div class="animate-pulse bg-muted rounded h-3 w-48"></div>
        </div>
      {/each}
    </div>

  <!-- ── Empty state ────────────────────────────────────────────────────────── -->
  {:else if jobs.length === 0}
    <div class="bg-card border border-border rounded-xl px-4 py-16 text-center">
      <div class="w-14 h-14 mx-auto mb-4 rounded-full bg-muted flex items-center justify-center">
        <svg class="w-7 h-7 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M12 6v6h4.5m4.5 0a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
      </div>
      <p class="text-sm font-semibold text-foreground mb-1">No cron jobs configured</p>
      <p class="text-xs text-muted-foreground mb-5">Schedule recurring tasks like cache warming, backups, or queue processing.</p>
      <button
        on:click={openCreate}
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors inline-flex items-center gap-2"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4" />
        </svg>
        Create First Cron Job
      </button>
    </div>

  <!-- ── Job cards ──────────────────────────────────────────────────────────── -->
  {:else}
    <div class="space-y-3">
      {#each jobs as job (job.id)}
        <div class="bg-card border border-border rounded-xl overflow-hidden">
          <div class="p-4">
            <div class="flex items-start gap-3">
              <!-- Left: info -->
              <div class="flex-1 min-w-0 space-y-1.5">
                <!-- Label + schedule badge -->
                <div class="flex items-center gap-2 flex-wrap">
                  <span class="text-sm font-semibold text-foreground">{job.label || 'Untitled'}</span>
                  <code class="inline-flex items-center px-2 py-0.5 rounded bg-muted text-foreground font-mono text-xs border border-border">
                    {job.schedule}
                  </code>
                  <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs bg-muted/60 text-muted-foreground border border-border">
                    {humanReadable(job.schedule)}
                  </span>
                </div>
                <!-- Command -->
                <p class="font-mono text-xs text-muted-foreground truncate max-w-xl" title={job.command}>
                  {job.command.length > 60 ? job.command.slice(0, 60) + '…' : job.command}
                </p>
                <!-- Last run + exit code -->
                <div class="flex items-center gap-3 flex-wrap">
                  <span class="text-xs text-muted-foreground">Last run: {formatDate(job.last_run_at)}</span>
                  {#if job.last_exit_code !== null}
                    {#if job.last_exit_code === 0}
                      <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-green-500/10 text-green-400 border border-green-500/20">
                        <svg class="w-2.5 h-2.5" fill="currentColor" viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>
                        Success
                      </span>
                    {:else}
                      <span class="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-red-500/10 text-red-400 border border-red-500/20">
                        <svg class="w-2.5 h-2.5" fill="currentColor" viewBox="0 0 8 8"><circle cx="4" cy="4" r="3"/></svg>
                        Failed (code {job.last_exit_code})
                      </span>
                    {/if}
                  {/if}
                </div>
              </div>

              <!-- Right: toggle + actions -->
              <div class="flex items-center gap-2 shrink-0 flex-wrap justify-end">
                <!-- Toggle switch with smooth transition -->
                <button
                  on:click={() => toggleJob(job)}
                  role="switch"
                  aria-checked={job.enabled}
                  title={job.enabled ? 'Disable' : 'Enable'}
                  class="relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2
                    {job.enabled ? 'bg-primary' : 'bg-muted'}"
                >
                  <span class="pointer-events-none inline-block h-4 w-4 rounded-full bg-background shadow-lg ring-0 transition-transform duration-200
                    {job.enabled ? 'translate-x-4' : 'translate-x-0'}"></span>
                </button>
                <!-- Run Now -->
                <button
                  on:click={() => runJob(job)}
                  disabled={runningJobId === job.id}
                  title="Run Now"
                  class="h-8 w-8 rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center justify-center"
                >
                  {#if runningJobId === job.id}
                    <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                      <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                      <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                    </svg>
                  {:else}
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M5.25 5.653c0-.856.917-1.398 1.667-.986l11.54 6.347a1.125 1.125 0 010 1.972l-11.54 6.347a1.125 1.125 0 01-1.667-.986V5.653z" />
                    </svg>
                  {/if}
                </button>
                <!-- Edit -->
                <button
                  on:click={() => openEdit(job)}
                  title="Edit"
                  class="h-8 w-8 rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground transition-colors inline-flex items-center justify-center"
                >
                  <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M16.862 4.487l1.687-1.688a1.875 1.875 0 112.652 2.652L6.832 19.82a4.5 4.5 0 01-1.897 1.13l-2.685.8.8-2.685a4.5 4.5 0 011.13-1.897L16.863 4.487z" />
                  </svg>
                </button>
                <!-- View Output -->
                <button
                  on:click={() => toggleOutput(job.id)}
                  title="View Output"
                  class="h-8 w-8 rounded-lg border transition-colors inline-flex items-center justify-center
                    {expandedOutputId === job.id ? 'border-primary/50 bg-primary/10 text-primary' : 'border-border text-muted-foreground hover:bg-muted hover:text-foreground'}"
                >
                  <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M6.75 7.5l3 2.25-3 2.25m4.5 0h3m-9 8.25h13.5A2.25 2.25 0 0021 18V6a2.25 2.25 0 00-2.25-2.25H5.25A2.25 2.25 0 003 6v12a2.25 2.25 0 002.25 2.25z" />
                  </svg>
                </button>
                <!-- Delete -->
                {#if deleteConfirmId === job.id}
                  <button
                    on:click={() => deleteJob(job.id)}
                    disabled={deleting}
                    class="h-8 px-2 rounded-lg bg-red-600 text-white text-xs font-medium hover:bg-red-700 transition-colors disabled:opacity-50 inline-flex items-center gap-1"
                  >
                    {#if deleting}
                      <svg class="w-3 h-3 animate-spin" fill="none" viewBox="0 0 24 24">
                        <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                        <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                      </svg>
                    {/if}
                    Confirm
                  </button>
                  <button
                    on:click={() => { deleteConfirmId = null; }}
                    class="h-8 px-2 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted transition-colors"
                  >Cancel</button>
                {:else}
                  <button
                    on:click={() => { deleteConfirmId = job.id; }}
                    title="Delete"
                    class="h-8 w-8 rounded-lg border border-border text-muted-foreground hover:bg-red-500/10 hover:text-red-400 hover:border-red-500/30 transition-colors inline-flex items-center justify-center"
                  >
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M14.74 9l-.346 9m-4.788 0L9.26 9m9.968-3.21c.342.052.682.107 1.022.166m-1.022-.165L18.16 19.673a2.25 2.25 0 01-2.244 2.077H8.084a2.25 2.25 0 01-2.244-2.077L4.772 5.79m14.456 0a48.108 48.108 0 00-3.478-.397m-12 .562c.34-.059.68-.114 1.022-.165m0 0a48.11 48.11 0 013.478-.397m7.5 0v-.916c0-1.18-.91-2.164-2.09-2.201a51.964 51.964 0 00-3.32 0c-1.18.037-2.09 1.022-2.09 2.201v.916m7.5 0a48.667 48.667 0 00-7.5 0" />
                    </svg>
                  </button>
                {/if}
              </div>
            </div>
          </div>

          <!-- Expandable output panel — terminal style -->
          {#if expandedOutputId === job.id}
            <div class="border-t border-border">
              <div class="bg-[#0d1117] flex items-center justify-between px-4 py-2 border-b border-border/30">
                <div class="flex items-center gap-2">
                  <span class="w-2.5 h-2.5 rounded-full bg-red-500/60"></span>
                  <span class="w-2.5 h-2.5 rounded-full bg-yellow-500/60"></span>
                  <span class="w-2.5 h-2.5 rounded-full bg-green-500/60"></span>
                </div>
                <span class="text-[10px] text-slate-500 font-mono">output — {job.label || 'Untitled'}</span>
                <div></div>
              </div>
              <pre class="bg-[#0d1117] text-green-400 font-mono text-xs p-4 max-h-48 overflow-y-auto whitespace-pre-wrap break-all rounded-b-xl">{outputMap[job.id] ?? 'No output available yet. Run the job to see output.'}</pre>
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<!-- ── Create / Edit Modal ─────────────────────────────────────────────────────── -->
{#if modalOpen}
  <!-- Backdrop -->
  <div
    class="fixed inset-0 z-40 bg-black/60 backdrop-blur-sm"
    on:click={closeModal}
    on:keydown={(e) => e.key === 'Escape' && closeModal()}
    role="button"
    tabindex="-1"
    aria-label="Close modal"
  ></div>

  <!-- Panel -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-labelledby="modal-title"
  >
    <div class="bg-card border border-border rounded-2xl w-full max-w-lg shadow-2xl overflow-hidden">
      <!-- Modal header -->
      <div class="flex items-center justify-between px-5 py-4 border-b border-border">
        <h3 id="modal-title" class="text-base font-semibold text-foreground flex items-center gap-2"><svg class="w-4 h-4 text-primary shrink-0" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>
          {editingJob ? 'Edit Cron Job' : 'New Cron Job'}
        </h3>
        <button
          on:click={closeModal}
          class="w-8 h-8 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground transition-colors inline-flex items-center justify-center"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <!-- Modal body -->
      <div class="px-5 py-5 space-y-4 max-h-[70vh] overflow-y-auto">
        <!-- Label -->
        <div class="space-y-1.5">
          <label class="block text-xs font-medium text-foreground" for="cron-label">Label</label>
          <input
            id="cron-label"
            bind:value={formLabel}
            placeholder="e.g. WordPress scheduler"
            class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>

        <!-- Schedule mode toggle -->
        <div class="space-y-3">
          <div class="flex items-center justify-between">
            <label class="block text-xs font-medium text-foreground">Schedule</label>
            <div class="flex rounded-lg border border-border overflow-hidden text-xs font-medium">
              <button
                on:click={() => { builderMode = true; syncRawFromBuilder(); }}
                class="px-3 py-1.5 transition-colors {builderMode ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-muted'}"
              >Visual</button>
              <button
                on:click={() => { builderMode = false; syncBuilderFromRaw(); }}
                class="px-3 py-1.5 transition-colors {!builderMode ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-muted'}"
              >Crontab</button>
            </div>
          </div>

          {#if builderMode}
            <!-- Visual builder -->
            <div class="grid grid-cols-5 gap-2">
              {#each [
                { label: 'Minute', bind: 'fMin', hint: '0-59' },
                { label: 'Hour', bind: 'fHr', hint: '0-23' },
                { label: 'Day', bind: 'fDom', hint: '1-31' },
                { label: 'Month', bind: 'fMon', hint: '1-12' },
                { label: 'Weekday', bind: 'fDow', hint: '0-6' },
              ] as field}
                <div class="space-y-1">
                  <label class="block text-[10px] font-medium text-muted-foreground">{field.label}</label>
                  {#if field.bind === 'fMin'}
                    <input bind:value={fMin} on:input={syncRawFromBuilder} placeholder="*" class="w-full h-9 px-2 rounded-lg border border-border bg-background text-sm text-center font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-ring" />
                  {:else if field.bind === 'fHr'}
                    <input bind:value={fHr} on:input={syncRawFromBuilder} placeholder="*" class="w-full h-9 px-2 rounded-lg border border-border bg-background text-sm text-center font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-ring" />
                  {:else if field.bind === 'fDom'}
                    <input bind:value={fDom} on:input={syncRawFromBuilder} placeholder="*" class="w-full h-9 px-2 rounded-lg border border-border bg-background text-sm text-center font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-ring" />
                  {:else if field.bind === 'fMon'}
                    <input bind:value={fMon} on:input={syncRawFromBuilder} placeholder="*" class="w-full h-9 px-2 rounded-lg border border-border bg-background text-sm text-center font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-ring" />
                  {:else}
                    <input bind:value={fDow} on:input={syncRawFromBuilder} placeholder="*" class="w-full h-9 px-2 rounded-lg border border-border bg-background text-sm text-center font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-ring" />
                  {/if}
                  <p class="text-[9px] text-muted-foreground text-center">{field.hint}</p>
                </div>
              {/each}
            </div>
            <p class="text-xs text-muted-foreground bg-muted/50 rounded-lg px-3 py-2">
              Runs: <span class="text-foreground font-medium">{humanReadable(computeSchedule())}</span>
              <span class="font-mono ml-2 text-[10px] opacity-70">{computeSchedule()}</span>
            </p>

          {:else}
            <!-- Crontab syntax -->
            <div class="space-y-2">
              <input
                bind:value={rawSchedule}
                placeholder="*/5 * * * *"
                class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
              />
              <p class="text-xs text-muted-foreground bg-muted/50 rounded-lg px-3 py-2">
                Runs: <span class="text-foreground font-medium">{humanReadable(rawSchedule)}</span>
              </p>
              <!-- Presets -->
              <div class="flex flex-wrap gap-1.5">
                {#each [
                  { label: 'Every minute', value: '* * * * *' },
                  { label: 'Every 5 min', value: '*/5 * * * *' },
                  { label: 'Every hour', value: '0 * * * *' },
                  { label: 'Daily 3am', value: '0 3 * * *' },
                  { label: 'Mon 9am', value: '0 9 * * 1' },
                ] as preset}
                  <button
                    on:click={() => applyPreset(preset.value)}
                    class="h-7 px-2.5 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground transition-colors
                      {rawSchedule === preset.value ? 'bg-muted text-foreground border-primary/40' : ''}"
                  >{preset.label}</button>
                {/each}
              </div>
            </div>
          {/if}
        </div>

        <!-- Command -->
        <div class="space-y-1.5">
          <label class="block text-xs font-medium text-foreground" for="cron-cmd">Command</label>
          <input
            id="cron-cmd"
            bind:value={formCommand}
            placeholder="php /var/www/vhosts/example.com/artisan schedule:run"
            class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm font-mono text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          />
        </div>

        <!-- Enabled -->
        <label class="flex items-center gap-3 cursor-pointer">
          <input type="checkbox" bind:checked={formEnabled} class="w-4 h-4 rounded border-border text-primary focus:ring-ring" />
          <span class="text-sm text-foreground">Enable this cron job</span>
        </label>
      </div>

      <!-- Modal footer -->
      <div class="flex items-center justify-end gap-2 px-5 py-4 border-t border-border bg-muted/30">
        <button
          on:click={closeModal}
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
        >Cancel</button>
        <button
          on:click={saveJob}
          disabled={saving || !formCommand.trim()}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed inline-flex items-center gap-2"
        >
          {#if saving}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
            </svg>
            Saving…
          {:else}
            {editingJob ? 'Save Changes' : 'Create Job'}
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ──────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div
    class="fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border transition-all duration-300
      {toast.type === 'success' ? 'bg-green-950/90 border-green-800 text-green-300' : 'bg-red-950/90 border-red-800 text-red-300'}"
    role="alert"
    aria-live="polite"
  >
    {#if toast.type === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
      </svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
      </svg>
    {/if}
    <span class="text-sm font-medium">{toast.message}</span>
    <button on:click={() => { toast = null; }} class="text-current opacity-60 hover:opacity-100 transition-opacity" aria-label="Dismiss">
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>
{/if}
