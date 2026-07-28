<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import PageHeader from '$components/ui/PageHeader.svelte';

  // ── Types ──────────────────────────────────────────────────────────────────
  interface ClientProfile {
    id: string;
    email: string;
    display_name: string | null;
    package: {
      name: string;
      max_sites: number;
      max_disk_gb: number;
      max_email_accounts: number;
      max_databases: number;
    } | null;
    usage: {
      sites_count: number;
      disk_used_gb: number;
      email_accounts: number;
      databases: number;
    };
  }

  // ── State ──────────────────────────────────────────────────────────────────
  let profile: ClientProfile | null = null;
  let loading = true;

  // Password change
  let currentPassword = '';
  let newPassword = '';
  let confirmPassword = '';
  let passwordError = '';
  let passwordLoading = false;
  let passwordSuccess = false;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';

  onMount(async () => {
    loading = true;
    try {
      profile = await api.client.profile();
    } catch {
      showToast('Failed to load profile', 'error');
    } finally {
      loading = false;
    }
  });

  async function changePassword(e: SubmitEvent) {
    e.preventDefault();
    passwordError = '';
    passwordSuccess = false;

    if (newPassword.length < 8) {
      passwordError = 'New password must be at least 8 characters.';
      return;
    }
    if (newPassword !== confirmPassword) {
      passwordError = 'Passwords do not match.';
      return;
    }

    passwordLoading = true;
    try {
      await api.client.changePassword(currentPassword, newPassword);
      currentPassword = '';
      newPassword = '';
      confirmPassword = '';
      passwordSuccess = true;
      showToast('Password changed successfully', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      passwordError = e.message ?? 'Failed to change password';
    } finally {
      passwordLoading = false;
    }
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function usagePct(used: number, max: number): number {
    if (max <= 0) return 0;
    return Math.min((used / max) * 100, 100);
  }

  function usageColor(pct: number): string {
    if (pct >= 90) return 'bg-red-500';
    if (pct >= 70) return 'bg-amber-500';
    return 'bg-primary';
  }
</script>

<svelte:head>
  <title>Profile — JottiCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-5">

  <PageHeader title="My Profile" subtitle="Account settings and resource usage" />

  {#if loading}
    <div class="animate-pulse space-y-3">
      {#each [1,2,3] as _}
        <div class="h-24 bg-muted rounded-xl"></div>
      {/each}
    </div>

  {:else if profile}

    <!-- Account Info -->
    <div class="bg-card border border-border rounded-xl p-4 space-y-3">
      <h2 class="text-sm font-semibold text-foreground">Account Information</h2>
      <div class="divide-y divide-border">
        {#each [
          { label: 'Email',        value: profile.email },
          { label: 'Display Name', value: profile.display_name ?? '—' },
          { label: 'User ID',      value: profile.id },
        ] as row}
          <div class="flex items-center py-2.5 text-sm first:pt-0 last:pb-0">
            <span class="w-32 shrink-0 text-muted-foreground">{row.label}</span>
            <span class="text-foreground font-mono text-xs">{row.value}</span>
          </div>
        {/each}
      </div>
      <a href="/settings/totp"
         class="inline-flex items-center gap-1.5 text-xs text-primary hover:underline font-medium">
        Manage Two-Factor Authentication
        <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M9 5l7 7-7 7" />
        </svg>
      </a>
    </div>

    <!-- Package Info + Usage Bars -->
    {#if profile.package}
      <div class="bg-card border border-border rounded-xl p-4 space-y-4">
        <div class="flex items-center justify-between">
          <h2 class="text-sm font-semibold text-foreground">Package: {profile.package.name}</h2>
          <span class="text-xs px-2 py-1 rounded-full bg-primary/10 text-primary font-medium">
            Active
          </span>
        </div>

        <div class="space-y-3">
          <!-- Sites -->
          <div>
            <div class="flex justify-between text-xs mb-1">
              <span class="text-muted-foreground">Websites</span>
              <span class="font-medium text-foreground">
                {profile.usage.sites_count} / {profile.package.max_sites}
              </span>
            </div>
            <div class="h-2 bg-muted rounded-full overflow-hidden">
              <div class="h-full rounded-full transition-all {usageColor(usagePct(profile.usage.sites_count, profile.package.max_sites))}"
                   style="width: {usagePct(profile.usage.sites_count, profile.package.max_sites)}%"></div>
            </div>
          </div>

          <!-- Disk -->
          <div>
            <div class="flex justify-between text-xs mb-1">
              <span class="text-muted-foreground">Disk Storage</span>
              <span class="font-medium text-foreground">
                {profile.usage.disk_used_gb.toFixed(1)} GB / {profile.package.max_disk_gb} GB
              </span>
            </div>
            <div class="h-2 bg-muted rounded-full overflow-hidden">
              <div class="h-full rounded-full transition-all {usageColor(usagePct(profile.usage.disk_used_gb, profile.package.max_disk_gb))}"
                   style="width: {usagePct(profile.usage.disk_used_gb, profile.package.max_disk_gb)}%"></div>
            </div>
          </div>

          <!-- Email Accounts -->
          <div>
            <div class="flex justify-between text-xs mb-1">
              <span class="text-muted-foreground">Email Accounts</span>
              <span class="font-medium text-foreground">
                {profile.usage.email_accounts} / {profile.package.max_email_accounts}
              </span>
            </div>
            <div class="h-2 bg-muted rounded-full overflow-hidden">
              <div class="h-full rounded-full transition-all {usageColor(usagePct(profile.usage.email_accounts, profile.package.max_email_accounts))}"
                   style="width: {usagePct(profile.usage.email_accounts, profile.package.max_email_accounts)}%"></div>
            </div>
          </div>

          <!-- Databases -->
          <div>
            <div class="flex justify-between text-xs mb-1">
              <span class="text-muted-foreground">Databases</span>
              <span class="font-medium text-foreground">
                {profile.usage.databases} / {profile.package.max_databases}
              </span>
            </div>
            <div class="h-2 bg-muted rounded-full overflow-hidden">
              <div class="h-full rounded-full transition-all {usageColor(usagePct(profile.usage.databases, profile.package.max_databases))}"
                   style="width: {usagePct(profile.usage.databases, profile.package.max_databases)}%"></div>
            </div>
          </div>
        </div>
      </div>
    {:else}
      <div class="bg-card border border-border rounded-xl p-4">
        <p class="text-sm text-muted-foreground">No package assigned. Contact your administrator.</p>
      </div>
    {/if}

    <!-- Change Password -->
    <div class="bg-card border border-border rounded-xl p-4 space-y-4">
      <h2 class="text-sm font-semibold text-foreground">Change Password</h2>

      <form on:submit={changePassword} class="space-y-3 max-w-sm">
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">Current Password</label>
          <input type="password" bind:value={currentPassword} required
                 class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm
                        text-foreground focus:outline-none focus:ring-2 focus:ring-ring" />
        </div>
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">New Password</label>
          <input type="password" bind:value={newPassword} required minlength="8"
                 placeholder="At least 8 characters"
                 class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm
                        text-foreground focus:outline-none focus:ring-2 focus:ring-ring" />
        </div>
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">Confirm New Password</label>
          <input type="password" bind:value={confirmPassword} required
                 class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm
                        text-foreground focus:outline-none focus:ring-2 focus:ring-ring" />
        </div>

        {#if passwordError}
          <p class="text-sm text-destructive">{passwordError}</p>
        {/if}
        {#if passwordSuccess}
          <p class="text-sm text-green-600">Password updated successfully.</p>
        {/if}

        <button type="submit" disabled={passwordLoading}
                class="h-10 px-6 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                       hover:bg-primary/90 transition-colors disabled:opacity-50">
          {passwordLoading ? 'Saving...' : 'Update Password'}
        </button>
      </form>
    </div>

  {/if}

</div>

<!-- Toast -->
{#if toastMessage}
  <div class="fixed bottom-6 right-4 z-50 flex items-center gap-2 px-4 py-3 rounded-lg shadow-lg text-sm font-medium
              {toastType === 'success' ? 'bg-green-600 text-white' : 'bg-destructive text-destructive-foreground'}"
       role="status" aria-live="polite">
    {toastMessage}
  </div>
{/if}
