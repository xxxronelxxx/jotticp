<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';

  // ── Types ─────────────────────────────────────────────────────────────────────

  interface MigrationJob {
    id:             string;
    user_id:        string;
    target_site_id: string | null;
    status:         'queued' | 'running' | 'completed' | 'failed';
    progress:       number;
    report:         MigrationReport | null;
    created_at:     string;
    completed_at:   string | null;
  }

  interface MigrationReport {
    domain?:        string;
    files_copied?:  number;
    db_imported?:   boolean;
    emails?:        number;
    dns_zones?:     number;
    cron_entries?:  number;
    errors?:        string[];
    warnings?:      string[];
    summary?:       string;
  }

  interface Toast { message: string; type: 'success' | 'error' }

  // ── State ─────────────────────────────────────────────────────────────────────

  type Source = 'cpanel' | 'plesk';
  let activeSource: Source = 'cpanel';

  // Upload state (shared)
  let fileInput: HTMLInputElement;
  let selectedFile: File | null = null;
  let uploading = false;
  let uploadProgress = 0; // 0-100, simulated

  // Jobs
  let jobs: MigrationJob[] = [];
  let jobsLoading = true;
  let pollInterval: ReturnType<typeof setInterval> | null = null;

  // Expanded job detail
  let expandedJobId: string | null = null;

  // Toast
  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Auth helpers ──────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const token = get(auth).token;
    return token ? { Authorization: 'Bearer ' + token } : {};
  }

  // ── Lifecycle ─────────────────────────────────────────────────────────────────

  onMount(async () => {
    await loadJobs();
    // Poll for running jobs every 5s
    pollInterval = setInterval(async () => {
      if (jobs.some(j => j.status === 'queued' || j.status === 'running')) {
        await loadJobs();
      }
    }, 5000);
  });

  onDestroy(() => {
    if (pollInterval) clearInterval(pollInterval);
  });

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function showToast(message: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 5000);
  }

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleString('en-US', {
      month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit',
    });
  }

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
    return (bytes / (1024 * 1024 * 1024)).toFixed(1) + ' GB';
  }

  function statusColor(status: string): string {
    if (status === 'completed') return 'bg-green-500/10 text-green-400 border-green-500/20';
    if (status === 'failed')    return 'bg-red-500/10 text-red-400 border-red-500/20';
    if (status === 'running')   return 'bg-blue-500/10 text-blue-400 border-blue-500/20';
    return 'bg-yellow-500/10 text-yellow-400 border-yellow-500/20';
  }

  // ── Data loading ──────────────────────────────────────────────────────────────

  async function loadJobs() {
    jobsLoading = true;
    try {
      const res = await fetch('/api/v1/migration/jobs', { headers: authH() });
      if (res.ok) jobs = await res.json() as MigrationJob[];
    } catch { /* ignore */ }
    finally { jobsLoading = false; }
  }

  // ── Import ────────────────────────────────────────────────────────────────────

  function onFileChange(e: Event) {
    const input = e.target as HTMLInputElement;
    selectedFile = input.files?.[0] ?? null;
  }

  async function startImport() {
    if (!selectedFile) return;
    uploading = true;
    uploadProgress = 0;

    const formData = new FormData();
    formData.append('archive', selectedFile);

    const endpoint = activeSource === 'cpanel'
      ? '/api/v1/migration/import'
      : '/api/v1/migration/import/plesk';

    // Simulate upload progress using XHR (fetch has no upload progress)
    try {
      const job = await uploadWithProgress(endpoint, formData);
      showToast(`Migration job started — tracking ID ${job.id.slice(0, 8)}`, 'success');
      selectedFile = null;
      fileInput.value = '';
      await loadJobs();
    } catch (err: unknown) {
      showToast((err as { message?: string }).message ?? 'Import failed', 'error');
    } finally {
      uploading = false;
      uploadProgress = 0;
    }
  }

  function uploadWithProgress(url: string, formData: FormData): Promise<MigrationJob> {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('POST', url);

      const token = get(auth).token;
      if (token) xhr.setRequestHeader('Authorization', 'Bearer ' + token);

      xhr.upload.addEventListener('progress', (e) => {
        if (e.lengthComputable) {
          uploadProgress = Math.round((e.loaded / e.total) * 100);
        }
      });

      xhr.addEventListener('load', () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          try {
            resolve(JSON.parse(xhr.responseText) as MigrationJob);
          } catch {
            reject(new Error('Invalid server response'));
          }
        } else {
          try {
            const body = JSON.parse(xhr.responseText) as { message?: string };
            reject(new Error(body.message ?? `HTTP ${xhr.status}`));
          } catch {
            reject(new Error(`HTTP ${xhr.status}`));
          }
        }
      });

      xhr.addEventListener('error', () => reject(new Error('Network error during upload')));
      xhr.send(formData);
    });
  }
</script>

<svelte:head>
  <title>Migration — JottiCP</title>
</svelte:head>

<div class="space-y-6 page-content">

  <!-- Header ───────────────────────────────────────────────────────────────── -->
  <div>
    <h1 class="text-2xl font-bold text-foreground tracking-tight">Migration Import</h1>
    <p class="text-sm text-muted-foreground mt-0.5">Import websites, databases, email accounts, and DNS from cPanel or Plesk</p>
  </div>

  <!-- Source selector ──────────────────────────────────────────────────────── -->
  <div class="inline-flex items-center gap-1 bg-muted rounded-xl p-1">
    {#each (['cpanel', 'plesk'] as const) as src}
      <button
        type="button"
        on:click={() => { activeSource = src; selectedFile = null; }}
        class="flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-all
               {activeSource === src
                 ? 'bg-card text-foreground shadow-sm border border-border'
                 : 'text-muted-foreground hover:text-foreground'}"
      >
        {#if src === 'cpanel'}
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4"/>
          </svg>
          cPanel
        {:else}
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2"/>
          </svg>
          Plesk
        {/if}
      </button>
    {/each}
  </div>

  <!-- Import card ──────────────────────────────────────────────────────────── -->
  <div class="grid grid-cols-1 lg:grid-cols-5 gap-6">

    <!-- Upload form (left, 3 cols) -->
    <div class="lg:col-span-3 bg-card border border-border rounded-xl p-6 space-y-5">
      <div class="flex items-center gap-3">
        <div class="w-9 h-9 rounded-xl bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9 13h6m-3-3v6m5 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>
          </svg>
        </div>
        <div>
          <h2 class="text-base font-semibold text-foreground">
            Upload {activeSource === 'cpanel' ? 'cPanel' : 'Plesk'} Backup
          </h2>
          <p class="text-xs text-muted-foreground">
            {activeSource === 'cpanel'
              ? 'Full cPanel backup (cpmove-*.tar.gz) or account backup'
              : 'Plesk backup archive (.tar or .tar.gz)'}
          </p>
        </div>
      </div>

      <!-- Drop zone -->
      <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
      <div
        class="border-2 border-dashed rounded-xl p-8 text-center cursor-pointer transition-colors
               {selectedFile ? 'border-primary/50 bg-primary/5' : 'border-border hover:border-primary/30 hover:bg-muted/30'}"
        on:click={() => fileInput.click()}
        on:dragover|preventDefault
        on:drop|preventDefault={(e) => {
          const f = e.dataTransfer?.files[0];
          if (f) { selectedFile = f; }
        }}
      >
        <input
          bind:this={fileInput}
          type="file"
          accept=".tar.gz,.tgz,.tar"
          class="hidden"
          on:change={onFileChange}
        />

        {#if selectedFile}
          <div class="flex flex-col items-center gap-2">
            <div class="w-12 h-12 rounded-full bg-primary/10 flex items-center justify-center">
              <svg class="w-6 h-6 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
                <path stroke-linecap="round" stroke-linejoin="round" d="M9 13h6m-3-3v6m5 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"/>
              </svg>
            </div>
            <p class="text-sm font-medium text-foreground">{selectedFile.name}</p>
            <p class="text-xs text-muted-foreground">{formatBytes(selectedFile.size)}</p>
            <button
              type="button"
              class="text-xs text-muted-foreground hover:text-foreground underline mt-1"
              on:click|stopPropagation={() => { selectedFile = null; fileInput.value = ''; }}
            >
              Remove
            </button>
          </div>
        {:else}
          <div class="flex flex-col items-center gap-3">
            <div class="w-12 h-12 rounded-full bg-muted flex items-center justify-center">
              <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                <path stroke-linecap="round" stroke-linejoin="round" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12"/>
              </svg>
            </div>
            <p class="text-sm font-medium text-foreground">Drop archive here or click to browse</p>
            <p class="text-xs text-muted-foreground">Max 2 GB · .tar.gz, .tgz, .tar</p>
          </div>
        {/if}
      </div>

      <!-- Upload progress bar -->
      {#if uploading}
        <div>
          <div class="flex items-center justify-between text-xs text-muted-foreground mb-1.5">
            <span>Uploading…</span>
            <span>{uploadProgress}%</span>
          </div>
          <div class="h-2 bg-muted rounded-full overflow-hidden">
            <div
              class="h-full bg-primary rounded-full transition-all duration-300"
              style="width: {uploadProgress}%"
            ></div>
          </div>
        </div>
      {/if}

      <button
        type="button"
        disabled={!selectedFile || uploading}
        on:click={startImport}
        class="w-full h-10 rounded-xl bg-primary text-primary-foreground text-sm font-medium
               hover:bg-primary/90 transition-all active:scale-[0.98]
               disabled:opacity-50 disabled:cursor-not-allowed
               inline-flex items-center justify-center gap-2"
      >
        {#if uploading}
          <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
          </svg>
          Uploading & Processing…
        {:else}
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12"/>
          </svg>
          Start {activeSource === 'cpanel' ? 'cPanel' : 'Plesk'} Import
        {/if}
      </button>
    </div>

    <!-- Info card (right, 2 cols) -->
    <div class="lg:col-span-2 space-y-4">

      <!-- What gets imported -->
      <div class="bg-card border border-border rounded-xl p-5">
        <h3 class="text-sm font-semibold text-foreground mb-3 flex items-center gap-2">
          <svg class="w-4 h-4 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
          </svg>
          What gets imported
        </h3>
        <ul class="space-y-2">
          {#each [
            { icon: 'M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z', label: 'Site files & docroot' },
            { icon: 'M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4', label: 'MySQL databases + users' },
            { icon: 'M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z', label: 'Email accounts & mailboxes' },
            { icon: 'M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9', label: 'DNS zones & records' },
            { icon: 'M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z', label: 'Cron jobs' },
          ] as item}
            <li class="flex items-center gap-2.5 text-sm text-muted-foreground">
              <svg class="w-4 h-4 text-primary/60 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                <path stroke-linecap="round" stroke-linejoin="round" d={item.icon}/>
              </svg>
              {item.label}
            </li>
          {/each}
        </ul>
      </div>

      <!-- How to get the backup -->
      <div class="bg-card border border-border rounded-xl p-5">
        <h3 class="text-sm font-semibold text-foreground mb-3">
          How to create a {activeSource === 'cpanel' ? 'cPanel' : 'Plesk'} backup
        </h3>
        {#if activeSource === 'cpanel'}
          <ol class="space-y-1.5 text-xs text-muted-foreground list-decimal list-inside leading-relaxed">
            <li>Log into cPanel → <strong class="text-foreground">Backup Wizard</strong></li>
            <li>Click <strong class="text-foreground">Full Backup</strong></li>
            <li>Choose <strong class="text-foreground">Download</strong> and wait for it to generate</li>
            <li>Download the <code class="font-mono bg-muted px-1 rounded">cpmove-*.tar.gz</code> file</li>
            <li>Upload it here</li>
          </ol>
          <p class="text-xs text-muted-foreground mt-3 italic">
            Alternatively, use <code class="font-mono bg-muted px-1 rounded">pkgacct</code> from WHM root.
          </p>
        {:else}
          <ol class="space-y-1.5 text-xs text-muted-foreground list-decimal list-inside leading-relaxed">
            <li>Log into Plesk → <strong class="text-foreground">Websites & Domains</strong></li>
            <li>Click <strong class="text-foreground">Backup Manager</strong></li>
            <li>Click <strong class="text-foreground">Back Up Now</strong></li>
            <li>Download the generated <code class="font-mono bg-muted px-1 rounded">.tar</code> file</li>
            <li>Upload it here</li>
          </ol>
          <p class="text-xs text-muted-foreground mt-3 italic">
            Also works with Plesk Migrator exported archives.
          </p>
        {/if}
      </div>
    </div>
  </div>

  <!-- Jobs list ─────────────────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden">
    <div class="px-5 py-4 border-b border-border flex items-center justify-between">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2"/>
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">Migration Jobs</h2>
      </div>
      <button
        on:click={loadJobs}
        class="h-8 px-3 rounded-lg border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground transition-colors inline-flex items-center gap-1.5"
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
        </svg>
        Refresh
      </button>
    </div>

    {#if jobsLoading && jobs.length === 0}
      <div class="divide-y divide-border">
        {#each [0, 1, 2] as _}
          <div class="flex items-center gap-4 px-5 py-4">
            <div class="animate-pulse bg-muted rounded-lg h-9 w-9 shrink-0"></div>
            <div class="flex-1 space-y-2">
              <div class="animate-pulse bg-muted rounded h-3.5 w-48"></div>
              <div class="animate-pulse bg-muted rounded h-2.5 w-64"></div>
            </div>
            <div class="animate-pulse bg-muted rounded-full h-5 w-20"></div>
          </div>
        {/each}
      </div>
    {:else if jobs.length === 0}
      <div class="px-5 py-12 text-center">
        <div class="w-12 h-12 mx-auto mb-3 rounded-full bg-muted flex items-center justify-center">
          <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4"/>
          </svg>
        </div>
        <p class="text-sm font-medium text-foreground mb-1">No migration jobs yet</p>
        <p class="text-xs text-muted-foreground">Upload a cPanel or Plesk backup above to get started</p>
      </div>
    {:else}
      <div class="divide-y divide-border">
        {#each jobs as job (job.id)}
          <div class="px-5 py-4">
            <!-- Job row -->
            <div class="flex items-center gap-4">
              <!-- Type icon -->
              <div class="w-9 h-9 rounded-lg bg-muted flex items-center justify-center shrink-0">
                <svg class="w-4.5 h-4.5 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4"/>
                </svg>
              </div>

              <!-- Info -->
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2 flex-wrap">
                  <span class="text-sm font-medium text-foreground font-mono">
                    {job.id.slice(0, 8)}…
                  </span>
                  {#if job.report?.domain}
                    <span class="text-xs text-muted-foreground">→ {job.report.domain}</span>
                  {/if}
                </div>
                <div class="flex items-center gap-3 mt-0.5">
                  <span class="text-xs text-muted-foreground">{formatDate(job.created_at)}</span>
                  {#if job.completed_at}
                    <span class="text-xs text-muted-foreground">· Completed {formatDate(job.completed_at)}</span>
                  {/if}
                </div>
                <!-- Progress bar for running/queued jobs -->
                {#if job.status === 'running' || job.status === 'queued'}
                  <div class="mt-2 h-1.5 bg-muted rounded-full overflow-hidden w-48">
                    <div
                      class="h-full bg-primary rounded-full transition-all duration-500
                             {job.status === 'queued' ? 'animate-pulse' : ''}"
                      style="width: {job.status === 'queued' ? 10 : job.progress}%"
                    ></div>
                  </div>
                  <span class="text-xs text-muted-foreground mt-0.5 block">
                    {job.status === 'queued' ? 'Queued…' : `${job.progress}% complete`}
                  </span>
                {/if}
              </div>

              <!-- Status badge -->
              <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border {statusColor(job.status)} capitalize shrink-0">
                {#if job.status === 'running'}
                  <svg class="w-2.5 h-2.5 mr-1 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                    <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                  </svg>
                {/if}
                {job.status}
              </span>

              <!-- Expand button (for completed jobs with reports) -->
              {#if (job.status === 'completed' || job.status === 'failed') && job.report}
                <button
                  type="button"
                  on:click={() => { expandedJobId = expandedJobId === job.id ? null : job.id; }}
                  class="h-8 w-8 rounded-lg border border-border text-muted-foreground hover:bg-muted hover:text-foreground transition-colors inline-flex items-center justify-center shrink-0"
                  aria-label="Toggle details"
                >
                  <svg
                    class="w-4 h-4 transition-transform {expandedJobId === job.id ? 'rotate-180' : ''}"
                    fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2"
                  >
                    <path stroke-linecap="round" stroke-linejoin="round" d="M19 9l-7 7-7-7"/>
                  </svg>
                </button>
              {/if}
            </div>

            <!-- Expanded report panel -->
            {#if expandedJobId === job.id && job.report}
              {@const r = job.report}
              <div class="mt-4 pl-13 space-y-4">
                <!-- Summary stats -->
                <div class="grid grid-cols-2 sm:grid-cols-5 gap-3">
                  {#each [
                    { label: 'Files', value: r.files_copied ?? 0 },
                    { label: 'Database', value: r.db_imported ? 'Yes' : 'No' },
                    { label: 'Email Accounts', value: r.emails ?? 0 },
                    { label: 'DNS Zones', value: r.dns_zones ?? 0 },
                    { label: 'Cron Jobs', value: r.cron_entries ?? 0 },
                  ] as stat}
                    <div class="bg-muted/40 rounded-lg px-3 py-2 text-center">
                      <div class="text-sm font-bold text-foreground">{stat.value}</div>
                      <div class="text-xs text-muted-foreground mt-0.5">{stat.label}</div>
                    </div>
                  {/each}
                </div>

                <!-- Summary text -->
                {#if r.summary}
                  <p class="text-sm text-foreground">{r.summary}</p>
                {/if}

                <!-- Warnings -->
                {#if r.warnings && r.warnings.length > 0}
                  <div class="bg-yellow-500/10 border border-yellow-500/20 rounded-lg p-3">
                    <p class="text-xs font-medium text-yellow-400 mb-1.5">Warnings</p>
                    <ul class="space-y-0.5">
                      {#each r.warnings as w}
                        <li class="text-xs text-yellow-300/80 flex items-start gap-1.5">
                          <svg class="w-3 h-3 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
                          </svg>
                          {w}
                        </li>
                      {/each}
                    </ul>
                  </div>
                {/if}

                <!-- Errors -->
                {#if r.errors && r.errors.length > 0}
                  <div class="bg-red-500/10 border border-red-500/20 rounded-lg p-3">
                    <p class="text-xs font-medium text-red-400 mb-1.5">Errors</p>
                    <ul class="space-y-0.5">
                      {#each r.errors as err}
                        <li class="text-xs text-red-300/80 flex items-start gap-1.5">
                          <svg class="w-3 h-3 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
                          </svg>
                          {err}
                        </li>
                      {/each}
                    </ul>
                  </div>
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>

</div>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(6px) } to { opacity:1; transform:none } }
  .page-content { animation: fadeUp 0.2s ease-out both }
</style>

<!-- ── Toast ──────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div
    class="fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border
      {toast.type === 'success'
        ? 'bg-green-950/90 border-green-800 text-green-300'
        : 'bg-red-950/90 border-red-800 text-red-300'}"
    role="alert"
    aria-live="polite"
  >
    {#if toast.type === 'success'}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
      </svg>
    {/if}
    <span class="text-sm font-medium">{toast.message}</span>
    <button on:click={() => toast = null} class="text-current opacity-60 hover:opacity-100 transition-opacity" aria-label="Dismiss">
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    </button>
  </div>
{/if}
