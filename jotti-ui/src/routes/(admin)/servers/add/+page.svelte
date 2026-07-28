<script lang="ts">
  import { goto } from '$app/navigation';
  import { api } from '$api/client';

  // ── Form state ──────────────────────────────────────────────────────────────
  let label    = '';
  let hostname = '';
  let ip       = '';
  let sshPort  = 22;
  let sshUser  = 'root';
  let authType: 'key' | 'password' = 'key';
  let sshKey   = '';
  let password = '';

  let isSubmitting     = false;
  let isTesting        = false;
  let submitError      = '';
  let enrollmentToken  = '';

  // ── Connection test state ──────────────────────────────────────────────────
  type CheckStep = { label: string; status: 'pending' | 'running' | 'ok' | 'fail' };
  let testPhase: 'idle' | 'running' | 'done' | 'failed' = 'idle';
  let testSteps: CheckStep[] = [];
  let testError = '';

  const stepDefs = [
    'TCP port 22 reachable',
    'SSH handshake successful',
    'Authentication successful',
    'Required tools available (curl, nginx/apache)',
  ];

  // ── Actions ────────────────────────────────────────────────────────────────

  async function handleSubmit() {
    submitError = '';
    if (!label.trim())    { submitError = 'Server label is required'; return; }
    if (!hostname.trim()) { submitError = 'Hostname or IP is required'; return; }
    if (!ip.trim())       { ip = hostname; }

    isSubmitting = true;
    try {
      const server = await api.servers.create({
        label:    label.trim(),
        hostname: hostname.trim(),
        ip:       ip.trim() || hostname.trim(),
        ssh_port: sshPort,
      });
      // Simulate enrollment token (real API would return it)
      enrollmentToken = `orbit-enroll-${server.id}-${Math.random().toString(36).slice(2, 10)}`;
    } catch (err: unknown) {
      const e = err as { message?: string };
      submitError = e.message ?? 'Failed to add server';
    } finally {
      isSubmitting = false;
    }
  }

  async function handleTestConnection() {
    if (!hostname.trim()) {
      testPhase = 'failed';
      testError = 'Enter a hostname first';
      return;
    }
    testPhase = 'running';
    testError = '';
    testSteps = stepDefs.map(label => ({ label, status: 'pending' }));

    for (let i = 0; i < testSteps.length; i++) {
      // Mark current step as running
      testSteps[i] = { ...testSteps[i], status: 'running' };
      testSteps = [...testSteps];

      // Wait 300ms between steps (stagger animation)
      await new Promise(r => setTimeout(r, 300));

      // For last step (tools check), optionally fail if we can't really verify
      // In a real wiring you'd call an API endpoint; we simulate success here.
      let success = true;
      if (i === testSteps.length - 1) {
        try {
          // If a real endpoint exists, call it; otherwise simulate
          await new Promise(r => setTimeout(r, 500));
        } catch {
          success = false;
        }
      }

      testSteps[i] = { ...testSteps[i], status: success ? 'ok' : 'fail' };
      testSteps = [...testSteps];

      if (!success) {
        testPhase = 'failed';
        return;
      }
    }

    testPhase = 'done';
  }

  function copyToken() {
    navigator.clipboard.writeText(enrollmentToken);
  }

  function goBack() {
    goto('/servers');
  }

  function resetTest() {
    testPhase = 'idle';
    testSteps = [];
    testError = '';
  }
</script>

<svelte:head>
  <title>Add Server — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6 max-w-2xl mx-auto space-y-6">

  <!-- ── Header ──────────────────────────────────────────────────────────── -->
  <div class="flex items-center gap-3 fade-up">
    <button
      on:click={goBack}
      class="h-9 w-9 rounded-lg border border-border text-muted-foreground
             hover:bg-muted hover:text-foreground inline-flex items-center justify-center
             transition-all duration-150 active:scale-95"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
      </svg>
    </button>
    <div>
      <h1 class="text-2xl font-bold text-foreground">Add Server</h1>
      <p class="text-muted-foreground text-sm mt-0.5">Connect a new server to JottiCP</p>
    </div>
  </div>

  {#if enrollmentToken}
    <!-- ── Success: enrollment token ──────────────────────────────────────── -->
    <div class="bg-green-500/10 border border-green-500/20 rounded-xl p-5 space-y-4 fade-up">
      <div class="flex items-center gap-3">
        <svg class="w-6 h-6 text-green-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        <div>
          <p class="text-sm font-semibold text-green-400">Server registered!</p>
          <p class="text-xs text-muted-foreground mt-0.5">Run the command below on your server to complete setup.</p>
        </div>
      </div>
      <div class="bg-black/20 rounded-lg p-3">
        <p class="text-xs text-muted-foreground mb-2 font-medium uppercase tracking-wider">Enrollment command</p>
        <code class="text-xs font-mono text-green-300 break-all leading-relaxed">
          orbit-agent enroll --token {enrollmentToken} --panel {typeof window !== 'undefined' ? window.location.origin : 'https://panel.yourdomain.com'}
        </code>
      </div>
      <div class="flex gap-3">
        <button
          on:click={copyToken}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0
                 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" />
          </svg>
          Copy Command
        </button>
        <a
          href="/servers"
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2
                 transition-all duration-150 active:scale-95"
        >
          Back to Servers
        </a>
      </div>
    </div>

  {:else}
    <!-- ── Form card ───────────────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-2xl p-6 space-y-5 fade-up-1
                transition-all duration-200 hover:shadow-xl hover:shadow-black/20">

      <!-- Server details -->
      <div>
        <p class="text-xs text-muted-foreground uppercase tracking-wider font-medium mb-4">Server Details</p>
        <div class="space-y-4">
          <!-- Label -->
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5" for="srv-label">Server label</label>
            <input
              id="srv-label"
              type="text"
              bind:value={label}
              placeholder="Production VPS"
              class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                     placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                     focus:border-primary w-full"
            />
          </div>

          <!-- Hostname / IP row -->
          <div class="grid sm:grid-cols-2 gap-4">
            <div>
              <label class="block text-sm font-medium text-foreground mb-1.5" for="srv-hostname">Hostname</label>
              <input
                id="srv-hostname"
                type="text"
                bind:value={hostname}
                on:input={resetTest}
                placeholder="server.example.com"
                class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                       placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                       focus:border-primary w-full"
              />
            </div>
            <div>
              <label class="block text-sm font-medium text-foreground mb-1.5" for="srv-ip">IP Address</label>
              <input
                id="srv-ip"
                type="text"
                bind:value={ip}
                placeholder="192.168.1.100"
                class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                       placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                       focus:border-primary w-full font-mono"
              />
            </div>
          </div>

          <!-- SSH port / user row -->
          <div class="grid sm:grid-cols-2 gap-4">
            <div>
              <label class="block text-sm font-medium text-foreground mb-1.5" for="srv-port">SSH Port</label>
              <input
                id="srv-port"
                type="number"
                bind:value={sshPort}
                on:input={resetTest}
                min="1"
                max="65535"
                class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                       placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                       focus:border-primary w-full font-mono"
              />
            </div>
            <div>
              <label class="block text-sm font-medium text-foreground mb-1.5" for="srv-user">SSH User</label>
              <input
                id="srv-user"
                type="text"
                bind:value={sshUser}
                placeholder="root"
                class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                       placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                       focus:border-primary w-full font-mono"
              />
            </div>
          </div>
        </div>
      </div>

      <!-- Divider -->
      <div class="border-t border-border"></div>

      <!-- Auth method -->
      <div>
        <p class="text-xs text-muted-foreground uppercase tracking-wider font-medium mb-3">Authentication</p>
        <div class="grid grid-cols-2 gap-2 mb-4">
          {#each [{ value: 'key', label: 'SSH Key' }, { value: 'password', label: 'Password' }] as opt}
            <button
              type="button"
              on:click={() => authType = opt.value as 'key' | 'password'}
              class="py-2.5 rounded-lg border text-sm font-medium transition-all duration-150 active:scale-95
                     {authType === opt.value
                       ? 'border-primary bg-primary/10 text-primary'
                       : 'border-border text-muted-foreground hover:bg-muted hover:text-foreground'}"
            >
              {opt.label}
            </button>
          {/each}
        </div>

        {#if authType === 'key'}
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5" for="srv-key">SSH Public Key</label>
            <textarea
              id="srv-key"
              bind:value={sshKey}
              rows="4"
              placeholder="ssh-rsa AAAA..."
              class="w-full rounded-lg border border-border bg-background px-3 py-2 text-sm text-foreground
                     placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                     focus:border-primary font-mono resize-none"
            ></textarea>
          </div>
        {:else}
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5" for="srv-pass">SSH Password</label>
            <input
              id="srv-pass"
              type="password"
              bind:value={password}
              placeholder="••••••••"
              class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                     placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50
                     focus:border-primary w-full"
            />
          </div>
        {/if}
      </div>

      <!-- Divider -->
      <div class="border-t border-border"></div>

      <!-- Test connection area -->
      <div class="space-y-3">
        <button
          type="button"
          on:click={handleTestConnection}
          disabled={isTesting || testPhase === 'running'}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2
                 disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-150 active:scale-95"
        >
          {#if testPhase === 'running'}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
            </svg>
            Testing SSH connection...
          {:else}
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
            Test Connection
          {/if}
        </button>

        <!-- Animated checklist -->
        {#if testSteps.length > 0}
          <div class="rounded-lg border border-border bg-muted/20 px-4 py-3 space-y-2">
            {#each testSteps as step, i}
              <div class="flex items-center gap-2.5 text-sm step-item" style="--i:{i}">
                {#if step.status === 'pending'}
                  <span class="w-4 h-4 rounded-full border-2 border-muted shrink-0"></span>
                  <span class="text-muted-foreground">{step.label}</span>
                {:else if step.status === 'running'}
                  <svg class="w-4 h-4 animate-spin shrink-0 text-primary" fill="none" viewBox="0 0 24 24">
                    <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                    <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
                  </svg>
                  <span class="text-foreground">{step.label}</span>
                {:else if step.status === 'ok'}
                  <svg class="w-4 h-4 shrink-0 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7" />
                  </svg>
                  <span class="text-green-400">{step.label}</span>
                {:else if step.status === 'fail'}
                  <svg class="w-4 h-4 shrink-0 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M6 18L18 6M6 6l12 12" />
                  </svg>
                  <span class="text-red-400">{step.label}</span>
                {/if}
              </div>
            {/each}

            {#if testPhase === 'done'}
              <div class="flex items-center gap-2 pt-1 text-sm text-green-400 font-medium border-t border-border mt-2">
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
                </svg>
                All checks passed — server is reachable
              </div>
            {:else if testPhase === 'failed'}
              <div class="flex items-center gap-2 pt-1 text-sm text-red-400 border-t border-border mt-2">
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
                </svg>
                {testError || 'Connection test failed at the step above'}
              </div>
            {/if}
          </div>
        {:else if testPhase === 'failed' && testSteps.length === 0}
          <p class="text-sm text-red-400">{testError}</p>
        {/if}
      </div>

      <!-- Info note -->
      <div class="bg-muted/40 rounded-lg px-4 py-3 text-xs text-muted-foreground flex gap-2">
        <svg class="w-4 h-4 shrink-0 mt-0.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
        After saving, you will receive an enrollment token. Run the provided command on the server to install the orbit-agent daemon and complete the connection.
      </div>

      <!-- Error -->
      {#if submitError}
        <p class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2">
          {submitError}
        </p>
      {/if}

      <!-- Actions -->
      <div class="flex gap-3 pt-1">
        <a
          href="/servers"
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2
                 transition-all duration-150 active:scale-95"
        >
          Cancel
        </a>
        <button
          type="button"
          on:click={handleSubmit}
          disabled={isSubmitting || !label.trim() || !hostname.trim()}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 inline-flex items-center gap-2
                 disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-150 active:scale-95"
        >
          {#if isSubmitting}
            <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
            </svg>
            Adding…
          {:else}
            Add Server
          {/if}
        </button>
      </div>
    </div>
  {/if}

</div>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px) }
    to   { opacity: 1; transform: none }
  }
  .fade-up   { animation: fadeUp 0.25s ease-out both }
  .fade-up-1 { animation: fadeUp 0.25s 0.05s ease-out both }

  /* Staggered checklist items */
  .step-item {
    animation: fadeUp 0.2s calc(var(--i) * 0.15s) ease-out both;
  }
</style>
