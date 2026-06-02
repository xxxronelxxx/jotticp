<script lang="ts">
  import { goto } from '$app/navigation';
  import { api } from '$api/client';
  import { auth } from '$stores/auth';
  import OrbitIcon from '$lib/components/ui/OrbitIcon.svelte';

  // ── State ──────────────────────────────────────────────────────────────────
  let currentStep = 1;
  const totalSteps = 6;

  // Step 2: Password change
  let newPassword = '';
  let confirmPassword = '';
  let passwordError = '';

  // Step 3: Panel domain
  let panelDomain = '';

  // Step 4: NS records
  let nsDomain = '';
  let detectedNs = 'ns1.registrar.com';
  let targetNs1 = '';
  let targetNs2 = '';
  let propagationStatus: { ok: boolean; detected: boolean } | null = null;
  let checkingPropagation = false;

  // Step 5: First site
  let siteDomain = '';
  let sitePhp = '8.3';
  let siteWebServer = 'ols';
  let siteCreating = false;
  let siteError = '';
  let siteCreated = false;

  // Loading
  let saving = false;
  let error = '';

  async function goToStep(step: number) {
    error = '';

    if (step === 3 && currentStep === 2) {
      // Validate password change
      if (!newPassword || newPassword.length < 8) {
        passwordError = 'Password must be at least 8 characters.';
        return;
      }
      if (newPassword !== confirmPassword) {
        passwordError = 'Passwords do not match.';
        return;
      }
      passwordError = '';
      saving = true;
      try {
        const userId = (await api.users.me()).id;
        await api.users.changePassword(userId, { new_password: newPassword });
      } catch (err: unknown) {
        const e = err as { message?: string };
        passwordError = e.message ?? 'Failed to change password';
        saving = false;
        return;
      } finally {
        saving = false;
      }
    }

    if (step === 4 && currentStep === 3) {
      // Save panel domain when advancing to NS records step
      if (panelDomain) {
        try {
          await api.settings.update({ panel_domain: panelDomain });
        } catch { /* non-critical */ }
      }
    }

    currentStep = step;
  }

  async function checkNsPropagation() {
    if (!nsDomain) return;
    checkingPropagation = true;
    propagationStatus = null;
    try {
      // Real propagation check via /api/v1/dns/propagation/:domain/NS
      const token = localStorage.getItem('orbit_access_token');
      const res = await fetch(`/api/v1/dns/propagation/${encodeURIComponent(nsDomain)}/NS`, {
        headers: token ? { Authorization: `Bearer ${token}` } : {},
      });
      if (res.ok) {
        const data = await res.json() as {
          propagated: boolean;
          detected_ns: string[];
          expected_ns: string[];
        };
        propagationStatus = { ok: data.propagated, detected: data.propagated };
        if (data.detected_ns?.length) {
          detectedNs = data.detected_ns.join(', ');
        }
      } else {
        // Fallback if endpoint not yet implemented
        propagationStatus = { ok: false, detected: false };
      }
    } catch {
      propagationStatus = { ok: false, detected: false };
    } finally {
      checkingPropagation = false;
    }
  }

  async function createFirstSite() {
    if (!siteDomain) return;
    siteCreating = true;
    siteError = '';
    try {
      await api.sites.create({ domain: siteDomain, php_version: sitePhp, web_server: siteWebServer });
      siteCreated = true;
    } catch (err: unknown) {
      const e = err as { message?: string };
      siteError = e.message ?? 'Failed to create site';
    } finally {
      siteCreating = false;
    }
  }

  async function completeWizard() {
    saving = true;
    try {
      await auth.markWizardComplete();
      await goto('/dashboard');
    } catch {
      error = 'Failed to complete setup. Please try again.';
    } finally {
      saving = false;
    }
  }
</script>

<svelte:head>
  <title>Setup — OrbitCP</title>
</svelte:head>

<!-- Full-page wizard (no sidebar) -->
<div class="min-h-screen bg-background flex flex-col items-center justify-center p-4">
  <div class="w-full max-w-lg">

    <!-- Brand header -->
    <div class="text-center mb-8">
      <div class="inline-flex items-center justify-center w-14 h-14 rounded-2xl bg-primary mb-4 shadow-xl shadow-indigo-500/25">
        <OrbitIcon size={32} className="text-white" />
      </div>
      <h1 class="text-2xl font-bold text-foreground">OrbitCP Setup</h1>
    </div>

    <!-- Progress bar -->
    <div class="mb-6">
      <div class="flex justify-between text-xs text-muted-foreground mb-1.5">
        <span>Step {currentStep} of {totalSteps}</span>
        <span>{Math.round((currentStep / totalSteps) * 100)}% complete</span>
      </div>
      <div class="h-1.5 bg-muted rounded-full overflow-hidden">
        <div class="h-full bg-[var(--accent)] rounded-full transition-all duration-500"
             style="width: {(currentStep / totalSteps) * 100}%"></div>
      </div>
    </div>

    <!-- Card -->
    <div class="bg-card border border-border rounded-2xl shadow-sm p-8">

      <!-- ── Step 1: Welcome ────────────────────────────────────────────── -->
      {#if currentStep === 1}
        <div class="text-center space-y-4">
          <div class="text-5xl">🚀</div>
          <h2 class="text-xl font-bold text-foreground">Welcome to OrbitCP!</h2>
          <p class="text-muted-foreground text-sm leading-relaxed">
            Let's get your panel ready in 3 minutes. We'll help you:
          </p>
          <ul class="text-sm text-muted-foreground space-y-2 text-left">
            {#each [
              'Set a secure admin password',
              'Configure your panel domain',
              'Set up nameserver records',
              'Add your first website',
            ] as item}
              <li class="flex items-center gap-2">
                <svg class="w-4 h-4 text-green-500 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7" />
                </svg>
                {item}
              </li>
            {/each}
          </ul>
          <button on:click={() => goToStep(2)}
                  class="w-full h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                         hover:bg-[var(--accent-hover)] transition-colors">
            Let's Get Started →
          </button>
        </div>

      <!-- ── Step 2: Password change ─────────────────────────────────────── -->
      {:else if currentStep === 2}
        <div class="space-y-4">
          <div>
            <h2 class="text-lg font-semibold text-foreground">Change Admin Password</h2>
            <p class="text-sm text-muted-foreground mt-1">
              Set a strong password for your admin account before continuing.
            </p>
          </div>

          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">New Password</label>
            <input type="password" bind:value={newPassword} required minlength="8"
                   placeholder="At least 8 characters"
                   class="w-full h-10 px-3 rounded-lg border border-border
                          bg-background text-sm text-foreground
                          focus:outline-none focus:ring-2 focus:ring-[var(--accent)]" />
          </div>
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">Confirm Password</label>
            <input type="password" bind:value={confirmPassword} required
                   class="w-full h-10 px-3 rounded-lg border border-border
                          bg-background text-sm text-foreground
                          focus:outline-none focus:ring-2 focus:ring-[var(--accent)]" />
          </div>

          {#if passwordError}
            <p class="text-sm text-red-500">{passwordError}</p>
          {/if}

          <div class="flex gap-3">
            <button on:click={() => goToStep(1)}
                    class="flex-1 h-10 rounded-lg border border-border text-sm
                           text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
              Back
            </button>
            <button on:click={() => goToStep(3)} disabled={saving}
                    class="flex-1 h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                           hover:bg-[var(--accent-hover)] transition-colors disabled:opacity-50">
              {saving ? 'Saving...' : 'Next →'}
            </button>
          </div>
        </div>

      <!-- ── Step 3: Panel domain ─────────────────────────────────────────── -->
      {:else if currentStep === 3}
        <div class="space-y-4">
          <div>
            <h2 class="text-lg font-semibold text-foreground">Panel Domain</h2>
            <p class="text-sm text-muted-foreground mt-1">
              Enter the domain or subdomain that points to this server. You'll use it to access OrbitCP.
            </p>
          </div>

          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">Panel Domain</label>
            <input type="text" bind:value={panelDomain}
                   placeholder="panel.yourdomain.com"
                   class="w-full h-10 px-3 rounded-lg border border-border
                          bg-background text-sm font-mono text-foreground
                          focus:outline-none focus:ring-2 focus:ring-[var(--accent)]" />
            <p class="text-xs text-muted-foreground mt-1.5">
              Make sure this domain's A record points to {typeof window !== 'undefined' ? window.location.hostname : 'this server'}.
            </p>
          </div>

          <div class="flex gap-3">
            <button on:click={() => goToStep(2)}
                    class="flex-1 h-10 rounded-lg border border-border text-sm
                           text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
              Back
            </button>
            <button on:click={() => goToStep(4)}
                    class="flex-1 h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                           hover:bg-[var(--accent-hover)] transition-colors">
              Next →
            </button>
          </div>
        </div>

      <!-- ── Step 4: NS records ──────────────────────────────────────────── -->
      {:else if currentStep === 4}
        <div class="space-y-4">
          <div>
            <h2 class="text-lg font-semibold text-foreground">Nameserver Records</h2>
            <p class="text-sm text-muted-foreground mt-1">
              Point your domain's nameservers to OrbitCP to manage DNS for your sites.
            </p>
          </div>

          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">
              Your domain name
            </label>
            <input type="text" bind:value={nsDomain} placeholder="yourdomain.com"
                   class="w-full h-10 px-3 rounded-lg border border-border
                          bg-background text-sm font-mono text-foreground
                          focus:outline-none focus:ring-2 focus:ring-[var(--accent)]" />
          </div>

          <div class="bg-muted/50 rounded-lg p-3 text-sm space-y-2">
            <div class="flex justify-between text-xs">
              <span class="text-muted-foreground">Current NS (detected):</span>
              <span class="font-mono text-foreground">{detectedNs}</span>
            </div>
            <div class="flex justify-between text-xs">
              <span class="text-muted-foreground">Target NS 1:</span>
              <span class="font-mono text-foreground">
                ns1.{panelDomain || 'yourdomain.com'}
              </span>
            </div>
            <div class="flex justify-between text-xs">
              <span class="text-muted-foreground">Target NS 2:</span>
              <span class="font-mono text-foreground">
                ns2.{panelDomain || 'yourdomain.com'}
              </span>
            </div>
          </div>

          <p class="text-xs text-muted-foreground/60">
            Log in to your domain registrar and change the nameservers to the values above. DNS changes may take up to 48 hours.
          </p>

          <!-- Propagation check -->
          <div class="flex items-center gap-2">
            <button on:click={checkNsPropagation} disabled={checkingPropagation || !nsDomain}
                    class="h-9 px-4 rounded-lg border border-border text-sm
                           text-muted-foreground hover:bg-muted hover:text-foreground
                           transition-colors disabled:opacity-50">
              {checkingPropagation ? 'Checking...' : 'Refresh Propagation'}
            </button>
            {#if propagationStatus}
              <span class="text-sm {propagationStatus.ok ? 'text-green-600' : 'text-amber-500'}">
                {propagationStatus.ok ? '✓ NS records propagated!' : '⏳ Still propagating...'}
              </span>
            {/if}
          </div>

          <div class="flex gap-3">
            <button on:click={() => goToStep(3)}
                    class="flex-1 h-10 rounded-lg border border-border text-sm
                           text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
              Back
            </button>
            <button on:click={() => goToStep(5)}
                    class="flex-1 h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                           hover:bg-[var(--accent-hover)] transition-colors">
              Next →
            </button>
          </div>
        </div>

      <!-- ── Step 5: First site ──────────────────────────────────────────── -->
      {:else if currentStep === 5}
        <div class="space-y-4">
          <div>
            <h2 class="text-lg font-semibold text-foreground">Add Your First Website</h2>
            <p class="text-sm text-muted-foreground mt-1">
              Create your first site — you can always add more later.
            </p>
          </div>

          {#if siteCreated}
            <div class="flex items-center gap-2 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-700
                        rounded-lg px-3 py-3 text-sm text-green-700 dark:text-green-400">
              <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M5 13l4 4L19 7" />
              </svg>
              Site <strong>{siteDomain}</strong> created! SSL will be issued automatically.
            </div>
          {:else}
            <div>
              <label class="block text-sm font-medium text-foreground mb-1.5">Domain</label>
              <input type="text" bind:value={siteDomain} placeholder="example.com"
                     class="w-full h-10 px-3 rounded-lg border border-border
                            bg-background text-sm font-mono text-foreground
                            focus:outline-none focus:ring-2 focus:ring-[var(--accent)]" />
            </div>
            <div class="grid grid-cols-2 gap-3">
              <div>
                <label class="block text-sm font-medium text-foreground mb-1.5">PHP Version</label>
                <select bind:value={sitePhp}
                        class="w-full h-10 px-3 rounded-lg border border-border
                               bg-background text-sm text-foreground
                               focus:outline-none focus:ring-2 focus:ring-[var(--accent)]">
                  {#each ['8.3', '8.2', '8.1', '8.0', '7.4'] as ver}
                    <option value={ver}>PHP {ver}</option>
                  {/each}
                </select>
              </div>
              <div>
                <label class="block text-sm font-medium text-foreground mb-1.5">Web Server</label>
                <select bind:value={siteWebServer}
                        class="w-full h-10 px-3 rounded-lg border border-border
                               bg-background text-sm text-foreground
                               focus:outline-none focus:ring-2 focus:ring-[var(--accent)]">
                  <option value="ols">OpenLiteSpeed</option>
                  <option value="nginx">Nginx</option>
                  <option value="apache">Apache</option>
                </select>
              </div>
            </div>

            {#if siteError}
              <p class="text-sm text-red-500">{siteError}</p>
            {/if}

            <button on:click={createFirstSite} disabled={siteCreating || !siteDomain}
                    class="w-full h-10 rounded-lg border border-border text-sm font-medium
                           text-foreground hover:bg-muted
                           transition-colors disabled:opacity-50">
              {siteCreating ? 'Creating...' : 'Create Site'}
            </button>
          {/if}

          <div class="flex gap-3">
            <button on:click={() => goToStep(4)}
                    class="flex-1 h-10 rounded-lg border border-border text-sm
                           text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
              Back
            </button>
            <button on:click={() => goToStep(6)}
                    class="flex-1 h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                           hover:bg-[var(--accent-hover)] transition-colors">
              {siteCreated ? 'Next →' : 'Skip for now →'}
            </button>
          </div>
        </div>

      <!-- ── Step 6: Done ────────────────────────────────────────────────── -->
      {:else if currentStep === 6}
        <div class="text-center space-y-5">
          <div class="text-5xl">🎉</div>
          <h2 class="text-xl font-bold text-foreground">Your Panel is Ready!</h2>
          <p class="text-sm text-muted-foreground">
            Here's what you can do next:
          </p>

          <div class="grid grid-cols-1 gap-3 text-left">
            {#each [
              { href: '/websites',  icon: '🌐', title: 'Add Websites',    desc: 'Create and manage hosted websites' },
              { href: '/servers',   icon: '🖥️', title: 'Add More Servers', desc: 'Expand to multiple servers'        },
              { href: '/settings/totp', icon: '🔐', title: 'Enable 2FA',  desc: 'Secure your admin account'         },
            ] as card}
              <a href={card.href}
                 class="flex items-start gap-3 p-3 rounded-xl border border-border
                        hover:bg-muted/50 hover:border-[var(--accent)]/50 transition-all">
                <span class="text-2xl">{card.icon}</span>
                <div>
                  <p class="text-sm font-semibold text-foreground">{card.title}</p>
                  <p class="text-xs text-muted-foreground mt-0.5">{card.desc}</p>
                </div>
              </a>
            {/each}
          </div>

          {#if error}
            <p class="text-sm text-red-500">{error}</p>
          {/if}

          <button on:click={completeWizard} disabled={saving}
                  class="w-full h-10 rounded-lg bg-[var(--accent)] text-white font-medium text-sm
                         hover:bg-[var(--accent-hover)] transition-colors disabled:opacity-50">
            {saving ? 'Finishing...' : 'Go to Dashboard →'}
          </button>
        </div>
      {/if}

    </div>
  </div>
</div>
