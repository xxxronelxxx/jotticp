<script lang="ts">
  import { onMount } from 'svelte';
  import { browser } from '$app/environment';
  import { goto } from '$app/navigation';
  import { page } from '$app/stores';
  import { auth, isAuthenticated } from '$stores/auth';
  import AdminLayout from '$components/layout/AdminLayout.svelte';
  import { isRTL } from '$lib/i18n';
  import OrbitIcon from '$lib/components/ui/OrbitIcon.svelte';

  let impersonatingAs = '';

  onMount(() => {
    auth.init();
    if (browser) {
      impersonatingAs = localStorage.getItem('orbit_impersonating') ?? '';
    }
  });

  function returnToAdmin() {
    if (!browser) return;
    const adminToken   = localStorage.getItem('orbit_original_token')   ?? '';
    const adminUser    = localStorage.getItem('orbit_original_user')     ?? '{}';
    const adminExpires = localStorage.getItem('orbit_original_expires')  ?? '0';

    if (!adminToken) return;

    localStorage.setItem('orbit_access_token',   adminToken);
    localStorage.setItem('orbit_user',           adminUser);
    localStorage.setItem('orbit_token_expires',  adminExpires);

    localStorage.removeItem('orbit_original_token');
    localStorage.removeItem('orbit_original_user');
    localStorage.removeItem('orbit_original_expires');
    localStorage.removeItem('orbit_impersonating');

    // Re-init auth store from restored session
    auth.init();
    impersonatingAs = '';
    goto('/reseller');
  }

  // Redirect unauthenticated users to login
  $: if ($auth.is_initialized && !$isAuthenticated) {
    goto('/login');
  }

  // Redirect to onboarding wizard if not complete (and not already there, and not impersonating)
  $: if (
    $auth.is_initialized &&
    $isAuthenticated &&
    $auth.user?.wizard_complete === false &&
    !$page.url.pathname.startsWith('/onboarding') &&
    !(browser && localStorage.getItem('orbit_impersonating'))
  ) {
    goto('/onboarding');
  }
</script>

{#if impersonatingAs}
  <div class="fixed top-0 inset-x-0 z-[999] flex items-center justify-between gap-4 px-4 py-2
              bg-amber-500 text-white text-sm font-medium shadow-lg">
    <span>
      👁 Viewing as <strong>{impersonatingAs}</strong> — admin features are restricted
    </span>
    <button
      on:click={returnToAdmin}
      class="flex items-center gap-1.5 px-3 py-1 rounded-lg bg-white/20 hover:bg-white/30
             transition-colors text-white text-xs font-semibold whitespace-nowrap"
    >
      ← Return to Admin
    </button>
  </div>
  <div class="h-9"></div>
{/if}

{#if $auth.is_initialized && $isAuthenticated}
  <div class="admin-layout" dir={$isRTL ? 'rtl' : 'ltr'}>
    <AdminLayout>
      <slot />
    </AdminLayout>
  </div>
{:else if !$auth.is_initialized}
  <!-- Loading state while auth initializes from localStorage -->
  <div class="min-h-screen flex items-center justify-center bg-[var(--bg-page)]">
    <div class="flex flex-col items-center gap-4">
      <div class="w-14 h-14 rounded-2xl bg-primary flex items-center justify-center shadow-xl shadow-indigo-500/25">
        <OrbitIcon size={32} className="text-white" />
      </div>
      <svg class="w-5 h-5 animate-spin text-[var(--accent)]" fill="none" viewBox="0 0 24 24">
        <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4" />
        <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
      </svg>
    </div>
  </div>
{/if}
