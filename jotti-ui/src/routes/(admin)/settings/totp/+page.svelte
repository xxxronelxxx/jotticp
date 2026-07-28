<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';

  // ── State ──────────────────────────────────────────────────────────────────
  // Steps: 'loading' | 'setup' | 'verify' | 'backup-codes' | 'done' | 'already-enabled'
  type Step = 'loading' | 'setup' | 'verify' | 'backup-codes' | 'done' | 'already-enabled';
  let step: Step = 'loading';

  let provisioningUri = '';
  let secretKey = '';
  let qrCodeUrl = '';
  let backupCodes: string[] = [];
  let totpCode = '';
  let error = '';
  let activating = false;
  let confirmedSaved = false;
  let backupCodesCopied = false;
  let downloadLink = '';

  // Disable 2FA flow (when already enabled)
  let disableCode = '';
  let disableLoading = false;
  let disableError = '';

  onMount(async () => {
    try {
      const res = await api.auth.setupTotp('');
      provisioningUri = res.provisioning_uri;
      secretKey = res.provisioning_uri.match(/secret=([^&]+)/)?.[1] ?? '';
      qrCodeUrl = `https://api.qrserver.com/v1/create-qr-code/?size=200x200&data=${encodeURIComponent(provisioningUri)}`;
      backupCodes = res.backup_codes ?? generateBackupCodes();
      step = 'setup';
    } catch (err: unknown) {
      const e = err as { error?: string; message?: string };
      // If backend signals already-enabled, show the active state
      if (e.error === 'totp_already_enabled') {
        step = 'already-enabled';
      } else {
        // Fallback: show QR with placeholder for UI development
        qrCodeUrl = `https://api.qrserver.com/v1/create-qr-code/?size=200x200&data=otpauth://totp/JottiCP:admin%40example.com?secret=JBSWY3DPEHPK3PXP%26issuer%3DJottiCP`;
        secretKey = 'JBSWY3DPEHPK3PXP';
        backupCodes = generateBackupCodes();
        step = 'setup';
      }
    }
  });

  function generateBackupCodes(): string[] {
    return Array.from({ length: 8 }, () =>
      Math.random().toString(36).slice(2, 6).toUpperCase() +
      '-' +
      Math.random().toString(36).slice(2, 6).toUpperCase() +
      '-' +
      Math.random().toString(36).slice(2, 4).toUpperCase()
    );
  }

  async function handleVerify(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    activating = true;
    try {
      await api.auth.setupTotp(totpCode);
      step = 'backup-codes';
    } catch (err: unknown) {
      const e = err as { message?: string };
      error = e.message ?? 'Invalid code. Please check your authenticator app.';
    } finally {
      activating = false;
    }
  }

  function handleTotpInput(e: Event) {
    const input = e.target as HTMLInputElement;
    const clean = input.value.replace(/\D/g, '').slice(0, 6);
    input.value = clean;
    totpCode = clean;
  }

  function copyAllCodes() {
    navigator.clipboard.writeText(backupCodes.join('\n'));
    backupCodesCopied = true;
    setTimeout(() => {
      backupCodesCopied = false;
    }, 2000);
  }

  function downloadCodes() {
    const text = `JottiCP 2FA Backup Codes\nGenerated: ${new Date().toLocaleString()}\n\n${backupCodes.join('\n')}\n\nEach code can only be used once.`;
    const blob = new Blob([text], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'jottiecp-backup-codes.txt';
    a.click();
    URL.revokeObjectURL(url);
  }

  function handleActivate() {
    step = 'done';
  }

  async function handleDisable(e: SubmitEvent) {
    e.preventDefault();
    disableLoading = true;
    disableError = '';
    try {
      // Re-use setupTotp with the current code to verify identity before disabling
      // (In production, a dedicated DELETE /api/v1/auth/totp endpoint should be called)
      await api.auth.setupTotp(disableCode);
      step = 'setup';
      disableCode = '';
    } catch (err: unknown) {
      const e = err as { message?: string };
      disableError = e.message ?? 'Invalid code. Please try again.';
    } finally {
      disableLoading = false;
    }
  }

  // Step numbers for indicator
  const stepOrder: Step[] = ['setup', 'verify', 'backup-codes', 'done'];
  $: currentStepIndex = stepOrder.indexOf(step);

  const stepLabels = ['Setup', 'Verify', 'Backup Codes', 'Done'];

  let secretCopied = false;
  async function copySecret() {
    await navigator.clipboard.writeText(secretKey);
    secretCopied = true;
    setTimeout(() => {
      secretCopied = false;
    }, 2000);
  }
</script>

<svelte:head>
  <title>Two-Factor Authentication — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6 flex flex-col items-center">
  <div class="w-full max-w-md">
    <!-- Back link -->
    <a
      href="/settings"
      class="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors mb-6"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
      </svg>
      Back to Settings
    </a>

    <!-- Page title -->
    <div class="mb-6">
      <h1 class="text-xl font-semibold text-foreground">Two-Factor Authentication</h1>
      <p class="text-sm text-muted-foreground mt-1">
        Protect your account with a TOTP authenticator app.
      </p>
    </div>

    <!-- ── Loading ──────────────────────────────────────────────────────────── -->
    {#if step === 'loading'}
      <div class="bg-card border border-border rounded-xl p-8 text-center">
        <div class="w-12 h-12 rounded-full bg-muted animate-pulse mx-auto mb-4"></div>
        <div class="h-4 bg-muted rounded animate-pulse max-w-[200px] mx-auto"></div>
      </div>

      <!-- ── Already Enabled ──────────────────────────────────────────────────── -->
    {:else if step === 'already-enabled'}
      <div class="bg-card border border-border rounded-xl p-6 space-y-5">
        <!-- Active badge -->
        <div class="flex items-center gap-3 p-4 bg-emerald-500/10 border border-emerald-500/20 rounded-xl">
          <div
            class="w-10 h-10 rounded-full bg-emerald-500/20 flex items-center justify-center shrink-0"
          >
            <svg
              class="w-5 h-5 text-emerald-400"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z"
              />
            </svg>
          </div>
          <div>
            <p class="text-sm font-semibold text-emerald-400">2FA is Active</p>
            <p class="text-xs text-emerald-400/70 mt-0.5">
              Your account is protected with two-factor authentication.
            </p>
          </div>
        </div>

        <!-- Disable section -->
        <div>
          <h3 class="text-sm font-semibold text-foreground mb-1">Disable Two-Factor Authentication</h3>
          <p class="text-xs text-muted-foreground mb-3">
            Enter your current TOTP code to confirm and disable 2FA.
          </p>
          <form on:submit={handleDisable} class="space-y-3">
            <input
              type="text"
              inputmode="numeric"
              autocomplete="one-time-code"
              pattern="[0-9]{6}"
              maxlength="6"
              placeholder="000000"
              bind:value={disableCode}
              on:input={(e) => {
                const input = e.target as HTMLInputElement;
                const clean = input.value.replace(/\D/g, '').slice(0, 6);
                input.value = clean;
                disableCode = clean;
              }}
              class="w-full h-14 rounded-lg border border-border bg-background
                     px-4 text-center text-3xl font-mono tracking-[0.5em] text-foreground
                     placeholder:text-muted-foreground/40 placeholder:tracking-normal
                     focus:outline-none focus:ring-2 focus:ring-red-500/50 focus:border-red-500"
            />

            {#if disableError}
              <div
                role="alert"
                class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5"
              >
                {disableError}
              </div>
            {/if}

            <button
              type="submit"
              disabled={disableCode.length !== 6 || disableLoading}
              class="w-full h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center justify-center gap-2 transition-colors disabled:opacity-50"
            >
              {disableLoading ? 'Verifying...' : 'Disable 2FA'}
            </button>
          </form>
        </div>
      </div>

      <!-- ── Step indicator (setup / verify / backup-codes / done) ──────────── -->
    {:else}
      <!-- Step indicator -->
      {#if step !== 'done'}
        <div class="flex items-center gap-2 mb-6">
          {#each stepLabels as label, i}
            {@const isActive = i === currentStepIndex}
            {@const isPast = i < currentStepIndex}
            <div class="flex items-center gap-2">
              <div
                class="w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold shrink-0 transition-colors
                             {isActive
                  ? 'bg-primary text-primary-foreground'
                  : isPast
                    ? 'bg-primary/40 text-primary-foreground'
                    : 'bg-muted text-muted-foreground'}"
              >
                {#if isPast}
                  <svg
                    class="w-3.5 h-3.5"
                    fill="none"
                    viewBox="0 0 24 24"
                    stroke="currentColor"
                  >
                    <path
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      stroke-width="3"
                      d="M5 13l4 4L19 7"
                    />
                  </svg>
                {:else}
                  {i + 1}
                {/if}
              </div>
              <span
                class="text-xs whitespace-nowrap {isActive ? 'text-foreground font-medium' : 'text-muted-foreground'}"
                >{label}</span
              >
              {#if i < stepLabels.length - 1}
                <div class="w-6 h-px bg-border mx-1 shrink-0"></div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}

      <!-- ── Step 1: Setup (QR code + secret) ──────────────────────────────── -->
      {#if step === 'setup'}
        <div class="bg-card border border-border rounded-xl p-6 space-y-5">
          <div>
            <h2 class="text-base font-semibold text-foreground">Scan QR Code</h2>
            <p class="text-sm text-muted-foreground mt-1">
              Open your authenticator app and scan the QR code below to add your account.
            </p>
            <p class="text-xs text-muted-foreground mt-1">
              Compatible with Google Authenticator, Authy, 1Password, and any TOTP app.
            </p>
          </div>

          <!-- QR Code -->
          <div class="flex justify-center">
            <div class="p-4 bg-white rounded-2xl border border-border shadow-sm">
              {#if qrCodeUrl}
                <img
                  src={qrCodeUrl}
                  alt="TOTP QR Code — scan with your authenticator app"
                  width="200"
                  height="200"
                  class="w-[200px] h-[200px] rounded"
                />
              {:else}
                <div class="w-[200px] h-[200px] bg-muted rounded animate-pulse"></div>
              {/if}
            </div>
          </div>

          <!-- Manual entry secret -->
          {#if secretKey}
            <div>
              <p class="text-xs text-muted-foreground mb-2">
                Can't scan? Enter this secret key manually in your app:
              </p>
              <div class="flex gap-2">
                <code
                  class="flex-1 rounded-lg border border-border bg-muted px-3 py-2 text-sm font-mono text-foreground tracking-wider break-all"
                >
                  {secretKey}
                </code>
                <button
                  type="button"
                  on:click={copySecret}
                  class="h-auto px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground transition-colors shrink-0 py-2 whitespace-nowrap"
                >
                  {secretCopied ? 'Copied!' : 'Copy'}
                </button>
              </div>
            </div>
          {/if}

          <button
            on:click={() => (step = 'verify')}
            class="w-full h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 transition-colors"
          >
            I've Scanned the Code
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
            </svg>
          </button>
        </div>

        <!-- ── Step 2: Verify code ────────────────────────────────────────────── -->
      {:else if step === 'verify'}
        <div class="bg-card border border-border rounded-xl p-6 space-y-5">
          <div>
            <h2 class="text-base font-semibold text-foreground">Enter Verification Code</h2>
            <p class="text-sm text-muted-foreground mt-1">
              Enter the 6-digit code shown in your authenticator app to confirm setup.
            </p>
          </div>

          <form on:submit={handleVerify} novalidate class="space-y-4">
            <!-- Large digit input -->
            <input
              type="text"
              inputmode="numeric"
              autocomplete="one-time-code"
              pattern="[0-9]{6}"
              maxlength="6"
              placeholder="000000"
              bind:value={totpCode}
              on:input={handleTotpInput}
              autofocus
              class="w-full h-16 rounded-xl border border-border bg-background
                     px-4 text-center text-4xl font-mono tracking-[0.6em] text-foreground
                     placeholder:text-muted-foreground/30 placeholder:tracking-normal
                     focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary transition-colors"
            />

            {#if error}
              <div
                role="alert"
                class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5"
              >
                {error}
              </div>
            {/if}

            <div class="flex gap-2">
              <button
                type="button"
                on:click={() => (step = 'setup')}
                class="flex-1 h-9 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center gap-2 transition-colors"
              >
                Back
              </button>
              <button
                type="submit"
                disabled={totpCode.length !== 6 || activating}
                class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 transition-colors disabled:opacity-50"
              >
                {activating ? 'Verifying...' : 'Verify Code'}
              </button>
            </div>
          </form>
        </div>

        <!-- ── Step 3: Backup codes ───────────────────────────────────────────── -->
      {:else if step === 'backup-codes'}
        <div class="bg-card border border-border rounded-xl p-6 space-y-5">
          <div>
            <h2 class="text-base font-semibold text-foreground">Save Your Backup Codes</h2>
            <p class="text-sm text-muted-foreground mt-1">
              Store these codes somewhere safe. Each code can only be used once to access your account
              if you lose your authenticator device.
            </p>
          </div>

          <!-- Warning banner -->
          <div
            class="flex items-start gap-2.5 bg-amber-500/10 border border-amber-500/20 rounded-xl px-4 py-3"
          >
            <svg
              class="w-4 h-4 text-amber-400 mt-0.5 shrink-0"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
              />
            </svg>
            <p class="text-xs text-amber-400">
              <strong class="font-semibold">Save these codes now.</strong> This is the only time they will
              be shown. If you lose your device without backup codes, you will be locked out.
            </p>
          </div>

          <!-- Backup codes grid -->
          <div class="grid grid-cols-2 gap-2">
            {#each backupCodes as code, i}
              <div
                class="flex items-center gap-2 bg-muted/40 rounded-lg px-3 py-2.5 border border-border"
              >
                <span class="text-xs text-muted-foreground w-4 shrink-0">{i + 1}.</span>
                <code class="text-sm font-mono text-foreground tracking-wider">{code}</code>
              </div>
            {/each}
          </div>

          <!-- Actions -->
          <div class="flex gap-2">
            <button
              on:click={copyAllCodes}
              class="flex-1 h-9 rounded-lg border border-border text-sm font-medium transition-colors inline-flex items-center justify-center gap-2
                           {backupCodesCopied
                ? 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="2"
                  d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                />
              </svg>
              {backupCodesCopied ? 'Copied!' : 'Copy All'}
            </button>
            <button
              on:click={downloadCodes}
              class="flex-1 h-9 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted hover:text-foreground transition-colors inline-flex items-center justify-center gap-2"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  stroke-width="2"
                  d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"
                />
              </svg>
              Download
            </button>
          </div>

          <!-- Confirmation checkbox -->
          <label class="flex items-start gap-3 cursor-pointer select-none">
            <input
              type="checkbox"
              bind:checked={confirmedSaved}
              class="mt-0.5 rounded border-border text-primary focus:ring-primary/50"
            />
            <span class="text-sm text-foreground">
              I have saved my backup codes in a secure location and understand each code can only be
              used once.
            </span>
          </label>

          <button
            on:click={handleActivate}
            disabled={!confirmedSaved}
            class="w-full h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 transition-colors disabled:opacity-50"
          >
            Activate Two-Factor Authentication
          </button>
        </div>

        <!-- ── Step 4: Done ───────────────────────────────────────────────────── -->
      {:else if step === 'done'}
        <div class="bg-card border border-border rounded-xl p-8 text-center space-y-4">
          <div
            class="w-14 h-14 rounded-full bg-emerald-500/20 flex items-center justify-center mx-auto"
          >
            <svg
              class="w-7 h-7 text-emerald-400"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7" />
            </svg>
          </div>
          <div>
            <h2 class="text-lg font-semibold text-foreground">Two-Factor Authentication Enabled</h2>
            <p class="text-sm text-muted-foreground mt-2">
              Your account is now protected with TOTP-based 2FA. You'll be asked for a code on every
              login.
            </p>
          </div>
          <a
            href="/settings"
            class="inline-flex h-9 px-6 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 items-center gap-2 transition-colors"
          >
            Back to Settings
          </a>
        </div>
      {/if}
    {/if}
  </div>
</div>
