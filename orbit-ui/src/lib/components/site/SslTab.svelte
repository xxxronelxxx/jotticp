<script lang="ts">
  import { onMount, createEventDispatcher } from 'svelte';
  import { get } from 'svelte/store';
  import { api } from '$api/client';
  import type { Site } from '$api/client';
  import { auth } from '$lib/stores/auth';

  // ── Props ─────────────────────────────────────────────────────────────────────

  export let siteId: string;
  export let site: Site;

  // ── Event dispatcher ──────────────────────────────────────────────────────────

  const dispatch = createEventDispatcher<{ siteUpdated: void }>();

  // ── Types ─────────────────────────────────────────────────────────────────────

  interface SslCert {
    status: 'active' | 'pending' | 'expired' | 'none';
    issued_at: string | null;
    expires_at: string | null;
    domains: string[];
  }

  interface Toast { message: string; type: 'success' | 'error' }

  // ── State ─────────────────────────────────────────────────────────────────────

  let sslCert: SslCert | null = null;
  let loading = true;
  let loadError = false;

  let issueLoading = false;
  let wildcardLoading = false;
  let revokeLoading = false;
  let uploadLoading = false;

  let showUpload = false;
  let showRevokeConfirm = false;
  let showWildcardTooltip = false;

  let certPem = '';
  let keyPem = '';
  let chainPem = '';

  let toast: Toast | null = null;
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // ── Auth helper ───────────────────────────────────────────────────────────────

  function authH(): Record<string, string> {
    const t = get(auth).token;
    return { 'Content-Type': 'application/json', ...(t ? { Authorization: 'Bearer ' + t } : {}) };
  }

  // ── Toast helper ──────────────────────────────────────────────────────────────

  function showToast(message: string, type: 'success' | 'error' = 'success') {
    if (toastTimer) clearTimeout(toastTimer);
    toast = { message, type };
    toastTimer = setTimeout(() => { toast = null; }, 4000);
  }

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function formatDate(iso: string): string {
    return new Date(iso).toLocaleDateString('en-US', { year: 'numeric', month: 'short', day: 'numeric' });
  }

  function daysLeft(expiresAt: string): number {
    return Math.ceil((new Date(expiresAt).getTime() - Date.now()) / 86400000);
  }

  function expiryBadgeClass(days: number): string {
    if (days > 30) return 'bg-green-500/10 text-green-400 border-green-500/20';
    if (days > 7)  return 'bg-yellow-500/10 text-yellow-400 border-yellow-500/20';
    return 'bg-red-500/10 text-red-400 border-red-500/20';
  }

  function effectiveStatus(): 'active' | 'pending' | 'expired' | 'none' {
    return sslCert?.status ?? (site.ssl_status as 'active' | 'pending' | 'expired' | 'none') ?? 'none';
  }

  function effectiveExpiresAt(): string | null {
    return sslCert?.expires_at ?? site.ssl_expires_at ?? null;
  }

  // ── Data loading ──────────────────────────────────────────────────────────────

  onMount(async () => {
    await loadSsl();
  });

  async function loadSsl() {
    loading = true;
    loadError = false;
    try {
      const res = await fetch(`/api/v1/ssl/certs/${siteId}/status`, { headers: authH() });
      if (!res.ok) throw new Error('fetch failed');
      sslCert = await res.json() as SslCert;
    } catch {
      // Fall back to site prop values
      sslCert = {
        status: (site.ssl_status as SslCert['status']) ?? 'none',
        issued_at: null,
        expires_at: site.ssl_expires_at ?? null,
        domains: [site.domain],
      };
      // Only show error if we also can't fall back
    } finally {
      loading = false;
    }
  }

  // ── Actions ───────────────────────────────────────────────────────────────────

  async function issueSSL() {
    issueLoading = true;
    try {
      await api.sites.issueSSL(siteId);
      showToast('SSL issuance started — this may take a moment', 'success');
      await loadSsl();
      dispatch('siteUpdated');
    } catch {
      showToast('Failed to issue SSL certificate', 'error');
    } finally {
      issueLoading = false;
    }
  }

  async function issueWildcard() {
    wildcardLoading = true;
    try {
      const res = await fetch(`/api/v1/ssl/certs/${siteId}/issue-wildcard`, {
        method: 'POST',
        headers: authH(),
        body: '{}',
      });
      if (!res.ok) throw new Error('failed');
      showToast('Wildcard SSL issuance started — add the DNS TXT record to complete', 'success');
      await loadSsl();
    } catch {
      showToast('Failed to issue wildcard certificate', 'error');
    } finally {
      wildcardLoading = false;
    }
  }

  async function revokeSSL() {
    revokeLoading = true;
    showRevokeConfirm = false;
    try {
      const res = await fetch(`/api/v1/ssl/certs/${siteId}`, {
        method: 'DELETE',
        headers: authH(),
      });
      if (!res.ok) throw new Error('failed');
      showToast('Certificate revoked', 'success');
      await loadSsl();
      dispatch('siteUpdated');
    } catch {
      showToast('Failed to revoke certificate', 'error');
    } finally {
      revokeLoading = false;
    }
  }

  async function uploadCert() {
    if (!certPem.trim() || !keyPem.trim()) {
      showToast('Certificate and Private Key are required', 'error');
      return;
    }
    uploadLoading = true;
    try {
      const res = await fetch(`/api/v1/ssl/certs/${siteId}/upload`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ cert_pem: certPem, key_pem: keyPem, chain_pem: chainPem || undefined }),
      });
      if (!res.ok) throw new Error('failed');
      showToast('Custom certificate uploaded successfully', 'success');
      certPem = '';
      keyPem = '';
      chainPem = '';
      showUpload = false;
      await loadSsl();
      dispatch('siteUpdated');
    } catch {
      showToast('Failed to upload certificate', 'error');
    } finally {
      uploadLoading = false;
    }
  }
</script>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  .card-hover { transition: all 0.2s ease; }
  .card-hover:hover { transform: translateY(-1px); box-shadow: 0 8px 30px rgba(0,0,0,0.2); }
</style>

<!-- ── Content ──────────────────────────────────────────────────────────────── -->
<div class="tab-content space-y-5">

  <!-- ── Certificate status hero card ─────────────────────────────────────────── -->
  {#if loading}
    <div class="bg-card border border-border rounded-xl p-5 space-y-4 animate-pulse">
      <div class="flex items-center gap-4">
        <div class="w-14 h-14 rounded-full bg-muted shrink-0"></div>
        <div class="flex-1 space-y-2">
          <div class="h-5 bg-muted rounded w-32"></div>
          <div class="h-3 bg-muted rounded w-48"></div>
        </div>
      </div>
      <div class="h-3 bg-muted rounded w-full"></div>
    </div>
  {:else}
    {@const status = effectiveStatus()}
    {@const expiresAt = effectiveExpiresAt()}
    {@const days = expiresAt ? daysLeft(expiresAt) : null}
    {@const daysTotal = 90}
    {@const daysProgress = days !== null ? Math.max(0, Math.min(days, daysTotal)) : 0}
    {@const circumference = 2 * Math.PI * 20}
    {@const dashOffset = circumference - (daysProgress / daysTotal) * circumference}

    <div class="bg-card border rounded-xl p-5 transition-all
      {status === 'active' && days !== null && days > 30
        ? 'border-green-500/20 shadow-lg shadow-green-500/10'
        : status === 'active' && days !== null && days <= 30
        ? 'border-amber-500/20 shadow-lg shadow-amber-500/10'
        : 'border-border'}">
      <!-- Status row -->
      <div class="flex items-start gap-4">
        <!-- SVG Donut ring for days remaining -->
        <div class="relative w-14 h-14 shrink-0 flex items-center justify-center">
          {#if status === 'active' && days !== null}
            <svg class="w-14 h-14 -rotate-90" viewBox="0 0 48 48">
              <circle cx="24" cy="24" r="20" fill="none" stroke="currentColor"
                class="text-muted/40" stroke-width="4"/>
              <circle cx="24" cy="24" r="20" fill="none"
                stroke="{days > 30 ? '#4ade80' : days > 7 ? '#fbbf24' : '#f87171'}"
                stroke-width="4"
                stroke-linecap="round"
                stroke-dasharray="{circumference}"
                stroke-dashoffset="{dashOffset}"
                class="transition-all duration-700"/>
            </svg>
            <div class="absolute inset-0 flex flex-col items-center justify-center">
              <span class="text-[10px] font-bold {days > 30 ? 'text-green-400' : days > 7 ? 'text-amber-400' : 'text-red-400'} leading-none">{days}</span>
              <span class="text-[8px] text-muted-foreground leading-none mt-0.5">days</span>
            </div>
            {#if days <= 30}
              <span class="absolute -top-0.5 -right-0.5 w-3 h-3 rounded-full bg-amber-400 animate-pulse border-2 border-card"></span>
            {/if}
          {:else}
            <div class="w-14 h-14 rounded-full flex items-center justify-center
              {status === 'pending' ? 'bg-yellow-500/10' :
               status === 'expired' ? 'bg-red-500/10' : 'bg-muted'}">
              {#if status === 'pending'}
                <svg class="w-7 h-7 text-yellow-400 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
                </svg>
              {:else if status === 'expired'}
                <svg class="w-7 h-7 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                </svg>
              {:else}
                <svg class="w-7 h-7 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M8 11V7a4 4 0 018 0m-4 8v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2z" />
                </svg>
              {/if}
            </div>
          {/if}
        </div>

        <!-- Info -->
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 flex-wrap mb-1">
            <h3 class="text-base font-semibold text-foreground">
              {status === 'active' ? 'Certificate Active' :
               status === 'pending' ? 'Certificate Pending' :
               status === 'expired' ? 'Certificate Expired' : 'No Certificate'}
            </h3>
            {#if status === 'active' && days !== null}
              <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border {expiryBadgeClass(days)}">
                {days > 0 ? `expires in ${days} days` : 'expires today'}
              </span>
            {/if}
          </div>

          <!-- SANs as tag badges -->
          {#if sslCert?.domains && sslCert.domains.length > 0}
            <div class="flex flex-wrap gap-1.5 mb-2">
              {#each sslCert.domains as domain}
                <span class="inline-flex items-center px-2 py-0.5 rounded bg-muted text-xs font-mono text-muted-foreground border border-border">
                  {domain}
                </span>
              {/each}
            </div>
          {:else}
            <p class="text-sm text-muted-foreground mb-1">
              <span class="font-medium text-foreground">{site.domain}</span>
            </p>
          {/if}

          <!-- Dates -->
          <div class="flex items-center gap-4 text-xs text-muted-foreground">
            {#if sslCert?.issued_at}
              <span>Issued {formatDate(sslCert.issued_at)}</span>
            {/if}
            {#if expiresAt}
              <span>Expires {formatDate(expiresAt)}</span>
            {/if}
            <span class="inline-flex items-center gap-1">
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
              </svg>
              Let's Encrypt
            </span>
          </div>
        </div>
      </div>

      <!-- Action buttons row -->
      <div class="flex items-center gap-2 flex-wrap mt-4 pt-4 border-t border-border">
        <!-- Issue / Renew -->
        <button
          on:click={issueSSL}
          disabled={issueLoading}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 transition-all active:scale-95 disabled:opacity-50 disabled:cursor-not-allowed
                 inline-flex items-center gap-2"
        >
          {#if issueLoading}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
              <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
              <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
            </svg>
            Issuing...
          {:else}
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
            {status === 'active' ? 'Renew Certificate' : 'Issue Certificate'}
          {/if}
        </button>

        <!-- Issue Wildcard -->
        <div class="relative">
          <button
            on:click={issueWildcard}
            on:mouseenter={() => showWildcardTooltip = true}
            on:mouseleave={() => showWildcardTooltip = false}
            disabled={wildcardLoading}
            class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
                   hover:bg-muted transition-colors disabled:opacity-50 disabled:cursor-not-allowed
                   inline-flex items-center gap-2"
          >
            {#if wildcardLoading}
              <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
              Issuing...
            {:else}
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z" />
              </svg>
              Issue Wildcard
            {/if}
          </button>
          {#if showWildcardTooltip}
            <div class="absolute bottom-full left-1/2 -translate-x-1/2 mb-2 px-3 py-1.5 rounded-lg
                        bg-popover border border-border text-xs text-muted-foreground whitespace-nowrap shadow-lg z-10">
              Requires DNS TXT record validation
              <div class="absolute top-full left-1/2 -translate-x-1/2 border-4 border-transparent border-t-border"></div>
            </div>
          {/if}
        </div>

        <!-- Revoke -->
        {#if status === 'active'}
          <button
            on:click={() => showRevokeConfirm = true}
            disabled={revokeLoading}
            class="h-9 px-4 rounded-lg text-sm font-medium text-red-400 hover:bg-red-500/10
                   transition-colors disabled:opacity-50 disabled:cursor-not-allowed
                   inline-flex items-center gap-2"
          >
            {#if revokeLoading}
              <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
              Revoking...
            {:else}
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                <path stroke-linecap="round" stroke-linejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
              </svg>
              Revoke
            {/if}
          </button>
        {/if}
      </div>
    </div>
  {/if}

  <!-- ── Custom certificate upload ──────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl overflow-hidden">
    <button
      on:click={() => showUpload = !showUpload}
      class="w-full flex items-center justify-between px-5 py-4 text-left hover:bg-muted/50 transition-colors"
    >
      <div class="flex items-center gap-2">
        <svg class="w-4 h-4 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
        </svg>
        <span class="text-sm font-medium text-foreground">Upload Custom Certificate</span>
      </div>
      <svg class="w-4 h-4 text-muted-foreground transition-transform {showUpload ? 'rotate-180' : ''}"
           fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M19 9l-7 7-7-7" />
      </svg>
    </button>

    {#if showUpload}
      <div class="px-5 pb-5 space-y-4 border-t border-border">
        <p class="text-xs text-muted-foreground pt-4">
          Paste your PEM-encoded certificate files below. The certificate and private key are required.
        </p>

        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground uppercase tracking-wider" for="cert-pem">
            Certificate (PEM) <span class="text-red-400">*</span>
          </label>
          <textarea
            id="cert-pem"
            bind:value={certPem}
            rows="5"
            placeholder="-----BEGIN CERTIFICATE-----&#10;...&#10;-----END CERTIFICATE-----"
            class="w-full px-3 py-2 rounded-lg border border-border bg-background text-sm text-foreground
                   font-mono focus:outline-none focus:ring-2 focus:ring-ring resize-y"
          ></textarea>
        </div>

        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground uppercase tracking-wider" for="key-pem">
            Private Key (PEM) <span class="text-red-400">*</span>
          </label>
          <textarea
            id="key-pem"
            bind:value={keyPem}
            rows="5"
            placeholder="-----BEGIN PRIVATE KEY-----&#10;...&#10;-----END PRIVATE KEY-----"
            class="w-full px-3 py-2 rounded-lg border border-border bg-background text-sm text-foreground
                   font-mono focus:outline-none focus:ring-2 focus:ring-ring resize-y"
          ></textarea>
        </div>

        <div class="space-y-1">
          <label class="text-xs font-medium text-muted-foreground uppercase tracking-wider" for="chain-pem">
            Certificate Chain (PEM) — Optional
          </label>
          <textarea
            id="chain-pem"
            bind:value={chainPem}
            rows="4"
            placeholder="-----BEGIN CERTIFICATE-----&#10;...&#10;-----END CERTIFICATE-----"
            class="w-full px-3 py-2 rounded-lg border border-border bg-background text-sm text-foreground
                   font-mono focus:outline-none focus:ring-2 focus:ring-ring resize-y"
          ></textarea>
        </div>

        <div class="flex items-center gap-2">
          <button
            on:click={uploadCert}
            disabled={uploadLoading}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                   hover:bg-primary/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed
                   inline-flex items-center gap-2"
          >
            {#if uploadLoading}
              <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
                <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
              </svg>
              Uploading...
            {:else}
              Upload Certificate
            {/if}
          </button>
          <button
            on:click={() => { showUpload = false; certPem = ''; keyPem = ''; chainPem = ''; }}
            class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
                   hover:bg-muted transition-colors"
          >
            Cancel
          </button>
        </div>
      </div>
    {/if}
  </div>

  <!-- ── Auto-renewal notice ─────────────────────────────────────────────────── -->
  <div class="flex items-start gap-3 px-4 py-3 rounded-xl bg-blue-500/5 border border-blue-500/20">
    <svg class="w-4 h-4 text-blue-400 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
      <path stroke-linecap="round" stroke-linejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
    </svg>
    <p class="text-xs text-blue-300">
      <span class="font-medium">Auto-renewal is enabled.</span>
      Certificates are renewed automatically 30 days before expiry.
    </p>
  </div>

</div>

<!-- ── Revoke confirmation dialog ─────────────────────────────────────────────── -->
{#if showRevokeConfirm}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm">
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-sm shadow-2xl">
      <div class="flex items-center gap-3 mb-3">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
          </svg>
        </div>
        <h3 class="text-base font-semibold text-foreground">Revoke Certificate?</h3>
      </div>
      <p class="text-sm text-muted-foreground mb-5">
        This will revoke the SSL certificate for <strong class="text-foreground">{site.domain}</strong>.
        Your site will no longer be accessible over HTTPS until a new certificate is issued.
      </p>
      <div class="flex items-center gap-2 justify-end">
        <button
          on:click={() => showRevokeConfirm = false}
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
                 hover:bg-muted transition-colors"
        >
          Cancel
        </button>
        <button
          on:click={revokeSSL}
          class="h-9 px-4 rounded-lg text-sm font-medium bg-red-600 text-white hover:bg-red-700 transition-colors
                 inline-flex items-center gap-2"
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
          </svg>
          Revoke Certificate
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ──────────────────────────────────────────────────────────────────── -->
{#if toast}
  <div class="fixed bottom-4 right-4 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-2xl border transition-all
    {toast.type === 'success' ? 'bg-green-950/90 border-green-800 text-green-300' : 'bg-red-950/90 border-red-800 text-red-300'}"
    role="alert" aria-live="polite">
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
    <button on:click={() => toast = null} class="ml-2 opacity-60 hover:opacity-100 transition-opacity" aria-label="Dismiss">
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>
{/if}
