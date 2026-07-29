<script lang="ts">
  import { goto } from '$app/navigation';
  import { auth } from '$stores/auth';
  import { t } from '$lib/i18n';
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { page } from '$app/stores';
  import OrbitIcon from '$lib/components/ui/OrbitIcon.svelte';

  // ── State ──────────────────────────────────────────────────────────────────
  type Step = 'credentials' | 'totp' | 'backup-code';

  let step: Step        = 'credentials';
  let email             = '';
  let password          = '';
  let totpCode          = '';
  let backupCode        = '';
  let interimToken      = '';
  let errorMessage      = '';
  let isLoading         = false;
  let showPassword      = false;

  // Session expiry warning from query param
  let sessionExpiredMsg = '';

  onMount(() => {
    const reason = $page.url.searchParams.get('reason');
    if (reason === 'session_expired') {
      sessionExpiredMsg = get(t)('auth.session_expired');
    }
    // Initialize auth from storage to detect if already logged in
    auth.init();
  });

  // Refs for focus management
  let emailInput: HTMLInputElement;
  let totpInput:  HTMLInputElement;
  let backupInput: HTMLInputElement;

  // ── Handlers ───────────────────────────────────────────────────────────────

  async function handleCredentials(e: SubmitEvent) {
    e.preventDefault();
    errorMessage = '';
    isLoading    = true;

    try {
      const response = await auth.login(email.toLowerCase().trim(), password);
      interimToken = response.interim_token;

      if (response.totp_required) {
        step = 'totp';
        setTimeout(() => totpInput?.focus(), 50);
      } else if (response.access_token && response.user_id && response.email && response.role) {
        // No TOTP — backend returned full session token directly
        auth.setSession({
          access_token:    response.access_token,
          expires_in:      response.expires_in ?? 28800,
          user_id:         response.user_id,
          email:           response.email,
          role:            response.role,
          wizard_complete: response.wizard_complete ?? false,
        });
        if (response.wizard_complete) {
          await goto('/dashboard');
        } else {
          await goto('/onboarding');
        }
      } else {
        errorMessage = get(t)('auth.login_failed');
      }
    } catch (err: unknown) {
      const e = err as { message?: string; error?: string };
      errorMessage = e.message ?? get(t)('auth.invalid_credentials');
    } finally {
      isLoading = false;
    }
  }

  async function handleTotp(e: SubmitEvent) {
    e.preventDefault();
    errorMessage = '';
    isLoading    = true;

    try {
      const response = await auth.verifyTotp(interimToken, totpCode.replace(/\D/g, ''));

      if (!response.wizard_complete) {
        await goto('/onboarding');
      } else {
        await goto('/dashboard');
      }
    } catch (err: unknown) {
      const e = err as { message?: string; error?: string };
      errorMessage = e.error === 'invalid_totp'
        ? get(t)('auth.invalid_totp')
        : e.message ?? get(t)('auth.verify_failed');
      totpCode = '';
      totpInput?.focus();
    } finally {
      isLoading = false;
    }
  }

  async function handleBackupCode(e: SubmitEvent) {
    e.preventDefault();
    errorMessage = '';
    isLoading    = true;

    try {
      const response = await auth.verifyBackupCode(interimToken, backupCode.trim().toUpperCase());

      if (!response.wizard_complete) {
        await goto('/onboarding');
      } else {
        await goto('/dashboard');
      }
    } catch (err: unknown) {
      const e = err as { message?: string; error?: string };
      errorMessage = e.error === 'invalid_totp'
        ? get(t)('auth.invalid_backup')
        : e.message ?? get(t)('auth.verify_failed');
      backupCode = '';
      backupInput?.focus();
    } finally {
      isLoading = false;
    }
  }

  function handleTotpInput(e: Event) {
    const input = e.target as HTMLInputElement;
    const clean = input.value.replace(/\D/g, '').slice(0, 6);
    input.value = clean;
    totpCode    = clean;
    if (clean.length === 6) {
      totpInput?.closest('form')?.requestSubmit();
    }
  }

  function goBackToCredentials() {
    step         = 'credentials';
    totpCode     = '';
    backupCode   = '';
    interimToken = '';
    errorMessage = '';
    setTimeout(() => emailInput?.focus(), 50);
  }

  function goToBackupCode() {
    step = 'backup-code';
    errorMessage = '';
    setTimeout(() => backupInput?.focus(), 50);
  }

  function goToTotp() {
    step = 'totp';
    errorMessage = '';
    setTimeout(() => totpInput?.focus(), 50);
  }
</script>

<svelte:head>
  <title>{$t('auth.login_title')} — JottiCP</title>
</svelte:head>

<!-- Light theme: white card on slate-50 background -->
<div class="min-h-screen flex items-center justify-center bg-background p-4">
  <div class="w-full max-w-md">

    <!-- Logo / brand header -->
    <div class="text-center mb-8">
      <div class="inline-flex items-center justify-center w-14 h-14 rounded-2xl
                  bg-primary mb-4 shadow-xl shadow-indigo-500/25">
        <OrbitIcon size={32} className="text-white" />
      </div>
      <h1 class="text-2xl font-bold text-foreground">JottiCP</h1>
      <p class="text-muted-foreground text-sm mt-1">{$t('auth.subtitle')}</p>
    </div>

    <!-- Session expired warning -->
    {#if sessionExpiredMsg}
      <div class="mb-4 flex items-center gap-2 rounded-lg bg-amber-50 border border-amber-200
                  dark:bg-amber-900/20 dark:border-amber-700 px-3 py-2.5 text-sm text-amber-800 dark:text-amber-400">
        <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
        </svg>
        {sessionExpiredMsg}
      </div>
    {/if}

    <!-- Login card -->
    <div class="bg-card border border-border
                rounded-2xl shadow-sm p-8">

      {#if step === 'credentials'}
        <!-- ── Step 1: Email + Password ────────────────────────────────── -->
        <div class="mb-6">
          <h2 class="text-lg font-semibold text-foreground">{$t('auth.login')}</h2>
          <p class="text-sm text-muted-foreground mt-1">
            {$t('auth.credentials_hint')}
          </p>
        </div>

        <form on:submit={handleCredentials} novalidate>
          <div class="space-y-4">

            <!-- Email -->
            <div>
              <label for="email" class="block text-sm font-medium text-foreground mb-1.5">
                {$t('auth.email')}
              </label>
              <input
                bind:this={emailInput}
                id="email"
                type="email"
                bind:value={email}
                autocomplete="email"
                autocapitalize="none"
                spellcheck="false"
                placeholder={$t('auth.email_placeholder')}
                required
                disabled={isLoading}
                class="w-full h-10 rounded-lg border border-border
                       bg-background px-3 py-2
                       text-sm text-foreground
                       placeholder:text-muted-foreground
                       focus:outline-none focus:ring-2 focus:ring-[var(--accent)] focus:border-transparent
                       disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
              />
            </div>

            <!-- Password -->
            <div>
              <div class="flex items-center justify-between mb-1.5">
                <label for="password" class="text-sm font-medium text-foreground">
                   {$t('auth.password')}
                </label>
                <a href="/forgot-password"
                   class="text-xs text-indigo-600 dark:text-indigo-400 hover:underline font-medium">
                  {$t('auth.forgot_password')}
                </a>
              </div>
              <div class="relative">
                <input
                  id="password"
                  type={showPassword ? 'text' : 'password'}
                  bind:value={password}
                  autocomplete="current-password"
                  required
                  disabled={isLoading}
                  class="w-full h-10 rounded-lg border border-border
                         bg-background px-3 py-2 pr-10
                         text-sm text-foreground
                         focus:outline-none focus:ring-2 focus:ring-[var(--accent)] focus:border-transparent
                         disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                />
                <button
                  type="button"
                  on:click={() => showPassword = !showPassword}
                  aria-label={showPassword ? $t('auth.hide_password') : $t('auth.show_password')}
                  class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground
                         hover:text-foreground transition-colors"
                >
                  {#if showPassword}
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7
                           a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878
                           9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3
                           3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543
                           7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                    </svg>
                  {:else}
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542
                           7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                    </svg>
                  {/if}
                </button>
              </div>
            </div>

            <!-- Error -->
            {#if errorMessage}
              <div role="alert"
                   class="flex items-start gap-2 rounded-lg bg-red-50 dark:bg-red-900/20
                          border border-red-200 dark:border-red-800
                          px-3 py-2.5 text-sm text-red-700 dark:text-red-400">
                <svg class="w-4 h-4 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667
                       1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34
                       16c-.77 1.333.192 3 1.732 3z" />
                </svg>
                <span>{errorMessage}</span>
              </div>
            {/if}

            <!-- Submit -->
            <button
              type="submit"
              disabled={isLoading || !email || !password}
              class="w-full h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                     hover:bg-[var(--accent-hover)] transition-colors
                     disabled:opacity-50 disabled:cursor-not-allowed
                     focus:outline-none focus:ring-2 focus:ring-[var(--accent)] focus:ring-offset-2
                     shadow-sm"
            >
              {#if isLoading}
                <span class="inline-flex items-center gap-2">
                  <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" />
                    <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                  </svg>
                  {$t('auth.signing_in')}
                </span>
              {:else}
                {$t('auth.continue')}
              {/if}
            </button>

          </div>
        </form>

      {:else if step === 'totp'}
        <!-- ── Step 2: TOTP ────────────────────────────────────────────── -->
        <div class="mb-6">
          <h2 class="text-lg font-semibold text-foreground">{$t('auth.totp_title')}</h2>
          <p class="text-sm text-muted-foreground mt-1">
            {$t('auth.totp_hint')}
          </p>
        </div>

        <form on:submit={handleTotp} novalidate>
          <div class="space-y-4">

            <!-- TOTP input -->
            <div>
              <label for="totp-code" class="block text-sm font-medium text-foreground mb-1.5">
                {$t('auth.totp_code')}
              </label>
              <input
                bind:this={totpInput}
                id="totp-code"
                type="text"
                inputmode="numeric"
                autocomplete="one-time-code"
                pattern="[0-9]{6}"
                maxlength="6"
                placeholder={$t('auth.totp_placeholder')}
                disabled={isLoading}
                on:input={handleTotpInput}
                class="w-full h-16 rounded-lg border border-border
                       bg-background
                       px-4 text-center text-3xl font-mono tracking-[0.5em] text-foreground
                       placeholder:text-muted-foreground/40 placeholder:tracking-normal
                       focus:outline-none focus:ring-2 focus:ring-[var(--accent)] focus:border-transparent
                       disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                aria-describedby="totp-help"
              />
              <p id="totp-help" class="text-xs text-muted-foreground mt-1.5">
                {$t('auth.totp_changes')}
              </p>
            </div>

            <!-- Error -->
            {#if errorMessage}
              <div role="alert"
                   class="flex items-start gap-2 rounded-lg bg-red-50 dark:bg-red-900/20
                          border border-red-200 dark:border-red-800
                          px-3 py-2.5 text-sm text-red-700 dark:text-red-400">
                <svg class="w-4 h-4 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732
                       4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                </svg>
                <span>{errorMessage}</span>
              </div>
            {/if}

            <!-- Verify button -->
            <button
              type="submit"
              disabled={isLoading || totpCode.length !== 6}
              class="w-full h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                     hover:bg-[var(--accent-hover)] transition-colors
                     disabled:opacity-50 disabled:cursor-not-allowed
                     focus:outline-none focus:ring-2 focus:ring-[var(--accent)] focus:ring-offset-2"
            >
              {#if isLoading}
                <span class="inline-flex items-center gap-2">
                  <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" />
                    <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                  </svg>
                  {$t('auth.verifying')}
                </span>
              {:else}
                {$t('auth.verify_signin')}
              {/if}
            </button>

            <!-- Navigation -->
            <div class="flex items-center justify-between text-sm">
              <button
                type="button"
                on:click={goBackToCredentials}
                class="text-muted-foreground hover:text-foreground transition-colors"
              >
                {$t('common.back')}
              </button>
              <button
                type="button"
                on:click={goToBackupCode}
                class="text-muted-foreground hover:text-foreground transition-colors text-xs"
              >
                {$t('auth.forgot_totp')}
              </button>
            </div>

          </div>
        </form>

      {:else}
        <!-- ── Step 3: Backup code ─────────────────────────────────────── -->
        <div class="mb-6">
          <h2 class="text-lg font-semibold text-foreground">{$t('auth.backup_title')}</h2>
          <p class="text-sm text-muted-foreground mt-1">
            {$t('auth.backup_hint')}
          </p>
        </div>

        <form on:submit={handleBackupCode} novalidate>
          <div class="space-y-4">

            <div>
              <label for="backup-code" class="block text-sm font-medium text-foreground mb-1.5">
                {$t('auth.backup_code')}
              </label>
              <input
                bind:this={backupInput}
                id="backup-code"
                type="text"
                bind:value={backupCode}
                autocomplete="off"
                autocapitalize="characters"
                spellcheck="false"
                placeholder={$t('auth.backup_placeholder')}
                maxlength="14"
                disabled={isLoading}
                class="w-full h-12 rounded-lg border border-border
                       bg-background
                       px-4 text-center text-xl font-mono tracking-widest text-foreground
                       placeholder:text-muted-foreground/40
                       focus:outline-none focus:ring-2 focus:ring-[var(--accent)] focus:border-transparent
                       disabled:opacity-50 disabled:cursor-not-allowed transition-colors uppercase"
              />
              <p class="text-xs text-muted-foreground mt-1.5">
                {$t('auth.backup_once')}
              </p>
            </div>

            <!-- Error -->
            {#if errorMessage}
              <div role="alert"
                   class="flex items-start gap-2 rounded-lg bg-red-50 dark:bg-red-900/20
                          border border-red-200 dark:border-red-800
                          px-3 py-2.5 text-sm text-red-700 dark:text-red-400">
                <svg class="w-4 h-4 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732
                       4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                </svg>
                <span>{errorMessage}</span>
              </div>
            {/if}

            <button
              type="submit"
              disabled={isLoading || backupCode.replace(/[^A-Za-z0-9]/g, '').length < 12}
              class="w-full h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                     hover:bg-[var(--accent-hover)] transition-colors
                     disabled:opacity-50 disabled:cursor-not-allowed
                     focus:outline-none focus:ring-2 focus:ring-[var(--accent)] focus:ring-offset-2"
            >
              {#if isLoading}
                <span class="inline-flex items-center gap-2">
                  <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                    <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" />
                    <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                  </svg>
                  {$t('auth.verifying')}
                </span>
              {:else}
                {$t('auth.backup_signin')}
              {/if}
            </button>

            <div class="flex items-center justify-between text-sm">
              <button
                type="button"
                on:click={goBackToCredentials}
                class="text-muted-foreground hover:text-foreground transition-colors"
              >
                {$t('common.back')}
              </button>
              <button
                type="button"
                on:click={goToTotp}
                class="text-muted-foreground hover:text-foreground transition-colors text-xs"
              >
                {$t('auth.use_totp')}
              </button>
            </div>

          </div>
        </form>
      {/if}

    </div>

    <!-- Footer -->
    <p class="text-center text-xs text-muted-foreground mt-6">
      JottiCP v{import.meta.env.VITE_JOTTICP_VERSION ?? '0.1.0'}
      &nbsp;·&nbsp;
      <a href="https://docs.jotticp.io" target="_blank" rel="noopener"
         class="hover:underline">{$t('common.documentation')}</a>
    </p>

  </div>
</div>
