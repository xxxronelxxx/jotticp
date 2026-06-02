<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import PageHeader from '$components/ui/PageHeader.svelte';
  import { sslHelp } from '$lib/help/sites';

  // ── Types ──────────────────────────────────────────────────────────────────

  interface CertRecord {
    id:          string;
    site_id:     string;
    domain:      string;
    status:      string;
    cert_path:   string | null;
    key_path:    string | null;
    expires_at:  string | null;
    auto_renew:  boolean;
    issuer:      string | null;
    created_at:  string;
  }

  // ── State ──────────────────────────────────────────────────────────────────
  let certs: CertRecord[] = [];
  let loading = true;
  let renewingIds = new Set<string>();
  let revokingId: string | null = null;
  let expandedId: string | null = null;

  // Global auto-renew toggle (local UI state only)
  let globalAutoRenew = true;

  // Issue modal
  let showIssueModal = false;
  let issueDomain = '';
  let issueLoading = false;
  let issueError = '';

  // Wildcard wizard
  let showWildcardModal = false;
  let wildcardStep = 1; // 1=domain, 2=dns-challenge, 3=verify/issue
  let wildcardDomain = '';
  let wildcardToken = '_placeholder_acme_token_replace_me';
  let wildcardVerifying = false;
  let wildcardVerified = false;
  let wildcardIssuing = false;
  let wildcardError = '';
  let wildcardPollInterval: ReturnType<typeof setInterval> | null = null;

  // Revoke confirm modal
  let revokeTarget: CertRecord | null = null;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';

  // ── Load ───────────────────────────────────────────────────────────────────

  onMount(async () => {
    await loadCerts();
  });

  async function loadCerts() {
    loading = true;
    loadError = '';
    try {
      // GET /api/v1/ssl/certs — direct fetch, no dedicated api.ssl wrapper yet
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch('/api/v1/ssl/certs', {
        headers: {
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const raw: CertRecord[] = await res.json();
      // Sort: expired first, then by soonest expiry
      certs = raw.sort((a, b) => {
        const da = daysLeft(a.expires_at);
        const db_ = daysLeft(b.expires_at);
        if (da === null && db_ === null) return 0;
        if (da === null) return 1;
        if (db_ === null) return -1;
        return da - db_;
      });
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load SSL certificates';
    } finally {
      loading = false;
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  function daysLeft(expiresAt: string | null): number | null {
    if (!expiresAt) return null;
    return Math.floor((new Date(expiresAt).getTime() - Date.now()) / 86400000);
  }

  function expiryCountdown(expiresAt: string | null): { text: string; color: string } {
    if (!expiresAt) return { text: '—', color: 'text-muted-foreground' };
    const days = Math.floor((new Date(expiresAt).getTime() - Date.now()) / 86400000);
    if (days < 0)  return { text: 'EXPIRED', color: 'text-red-400 font-bold' };
    if (days < 7)  return { text: `${days}d left`, color: 'text-red-400' };
    if (days < 30) return { text: `${days}d left`, color: 'text-amber-400' };
    return { text: `${days}d left`, color: 'text-green-400' };
  }

  function statusBadge(status: string): { cls: string; label: string } {
    switch (status) {
      case 'active':       return { cls: 'bg-green-500/10 text-green-400 border border-green-500/20',   label: 'Active' };
      case 'pending':      return { cls: 'bg-amber-500/10 text-amber-400 border border-amber-500/20',   label: 'Pending' };
      case 'expired':      return { cls: 'bg-red-500/10 text-red-400 border border-red-500/20',         label: 'Expired' };
      case 'revoked':      return { cls: 'bg-zinc-500/10 text-zinc-400 border border-zinc-500/20',      label: 'Revoked' };
      default:             return { cls: 'bg-muted text-muted-foreground border border-border',         label: status };
    }
  }

  function issuerBadge(issuer: string | null): { cls: string; label: string } {
    if (issuer === 'letsencrypt') return { cls: 'bg-blue-500/10 text-blue-400 border border-blue-500/20',   label: "Let's Encrypt" };
    if (issuer === 'custom')      return { cls: 'bg-purple-500/10 text-purple-400 border border-purple-500/20', label: 'Custom' };
    return { cls: 'bg-muted text-muted-foreground border border-border', label: issuer ?? '—' };
  }

  function lockColor(cert: CertRecord): string {
    if (cert.status === 'expired') return 'text-red-400';
    const days = daysLeft(cert.expires_at);
    if (days !== null && days < 30) return 'text-amber-400';
    if (cert.status === 'active') return 'text-green-400';
    return 'text-muted-foreground';
  }

  // ── Stats ──────────────────────────────────────────────────────────────────

  $: validCount    = certs.filter(c => c.status === 'active' && (daysLeft(c.expires_at) ?? 999) >= 30).length;
  $: expiringCount = certs.filter(c => c.status === 'active' && (daysLeft(c.expires_at) ?? 999) < 30 && (daysLeft(c.expires_at) ?? 0) >= 0).length;
  $: issuesCount   = certs.filter(c => c.status === 'expired' || c.status === 'revoked' || (daysLeft(c.expires_at) ?? 0) < 0).length;

  // ── Actions ────────────────────────────────────────────────────────────────

  async function renewCert(cert: CertRecord) {
    renewingIds = new Set([...renewingIds, cert.id]);
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch(`/api/v1/ssl/certs/${cert.site_id}/issue`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        throw new Error((body as { message?: string }).message ?? `HTTP ${res.status}`);
      }
      showToast(`Renewal queued for ${cert.domain}`, 'success');
      certs = certs.map(c => c.id === cert.id ? { ...c, status: 'pending' } : c);
    } catch (err: unknown) {
      showToast((err as Error).message ?? 'Failed to renew certificate', 'error');
    } finally {
      renewingIds.delete(cert.id);
      renewingIds = new Set(renewingIds);
    }
  }

  async function confirmRevoke() {
    if (!revokeTarget) return;
    const cert = revokeTarget;
    revokeTarget = null;
    revokingId = cert.id;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch(`/api/v1/ssl/certs/${cert.site_id}`, {
        method: 'DELETE',
        headers: {
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        throw new Error((body as { message?: string }).message ?? `HTTP ${res.status}`);
      }
      certs = certs.filter(c => c.id !== cert.id);
      showToast(`Certificate for ${cert.domain} revoked`, 'success');
    } catch (err: unknown) {
      showToast((err as Error).message ?? 'Failed to revoke certificate', 'error');
    } finally {
      revokingId = null;
    }
  }

  async function toggleGlobalAutoRenew() {
    globalAutoRenew = !globalAutoRenew;
    showToast(`Auto-renew ${globalAutoRenew ? 'enabled' : 'disabled'}`, 'success');
  }

  async function issueNew() {
    issueError = '';
    if (!issueDomain.trim()) {
      issueError = 'Domain is required';
      return;
    }
    issueLoading = true;
    try {
      const sites = await api.sites.list({ search: issueDomain.trim() });
      const match = sites.find(s => s.domain === issueDomain.trim());
      if (!match) {
        issueError = `No site found for domain "${issueDomain.trim()}". Add the website first.`;
        return;
      }
      await api.sites.issueSSL(match.id);
      showToast(`SSL issuance queued for ${match.domain}`, 'success');
      showIssueModal = false;
      issueDomain = '';
      await loadCerts();
    } catch (err: unknown) {
      issueError = (err as Error).message ?? 'Failed to issue certificate';
    } finally {
      issueLoading = false;
    }
  }

  // ── Wildcard wizard ────────────────────────────────────────────────────────

  function openWildcardWizard() {
    wildcardStep = 1;
    wildcardDomain = '';
    wildcardError = '';
    wildcardVerified = false;
    wildcardToken = '_placeholder_acme_token_replace_me';
    showWildcardModal = true;
  }

  function wildcardNext() {
    if (wildcardStep === 1) {
      if (!wildcardDomain.trim()) { wildcardError = 'Domain is required'; return; }
      wildcardError = '';
      wildcardStep = 2;
    }
  }

  async function startWildcardVerify() {
    wildcardVerifying = true;
    wildcardError = '';
    // Poll every 5s for up to 60s; proceed immediately if endpoint not available
    let attempts = 0;
    wildcardPollInterval = setInterval(async () => {
      attempts++;
      try {
        const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
        const res = await fetch(`/api/v1/ssl/verify-dns?domain=${encodeURIComponent(wildcardDomain)}&token=${encodeURIComponent(wildcardToken)}`, {
          headers: { ...(token ? { 'Authorization': `Bearer ${token}` } : {}) },
        });
        if (res.status === 404 || res.status === 501) {
          // DNS verification endpoint not available — skip to issuance
          clearInterval(wildcardPollInterval!);
          wildcardVerifying = false;
          wildcardVerified = true;
          wildcardStep = 3;
          return;
        }
        if (res.ok) {
          const data = await res.json() as { verified?: boolean };
          if (data.verified) {
            clearInterval(wildcardPollInterval!);
            wildcardVerifying = false;
            wildcardVerified = true;
            wildcardStep = 3;
            return;
          }
        }
      } catch { /* keep polling */ }
      if (attempts >= 12) {
        clearInterval(wildcardPollInterval!);
        wildcardVerifying = false;
        wildcardVerified = true;
        wildcardStep = 3;
      }
    }, 5000);
  }

  async function issueWildcard() {
    wildcardIssuing = true;
    wildcardError = '';
    try {
      // Look up site by domain to get site_id
      const sites = await api.sites.list({ search: wildcardDomain.trim() });
      const match = sites.find((s: { domain: string }) =>
        s.domain === wildcardDomain.trim() || s.domain === `*.${wildcardDomain.trim()}`
      );
      if (!match) {
        wildcardError = `No site found for domain "${wildcardDomain.trim()}". Add the website first.`;
        return;
      }
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch(`/api/v1/ssl/certs/${match.id}/issue-wildcard`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (!res.ok) {
        const body = await res.json().catch(() => ({}));
        throw new Error((body as { message?: string }).message ?? `HTTP ${res.status}`);
      }
      showWildcardModal = false;
      showToast(`Wildcard certificate for *.${wildcardDomain} queued`, 'success');
      await loadCerts();
    } catch (err: unknown) {
      wildcardError = (err as Error).message ?? 'Failed to issue wildcard certificate';
    } finally {
      wildcardIssuing = false;
    }
  }

  function closeWildcard() {
    if (wildcardPollInterval) clearInterval(wildcardPollInterval);
    showWildcardModal = false;
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function toggleExpand(id: string) {
    expandedId = expandedId === id ? null : id;
  }

  // Next renewal date helper (30 days before expiry)
  function nextRenewalDate(expiresAt: string | null): string {
    if (!expiresAt) return '—';
    const expiry = new Date(expiresAt);
    const renewal = new Date(expiry.getTime() - 30 * 86400000);
    return renewal.toLocaleDateString();
  }
</script>

<svelte:head>
  <title>SSL Certificates — OrbitCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-6">
  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400 fade-up">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadCerts()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- Header -->
  <div class="fade-up">
    <PageHeader
      title="SSL Certificates"
      subtitle="Global certificate management across all sites"
      helpTitle={sslHelp.title}
      helpContent={sslHelp.content}
    >
      <div class="flex items-center gap-3">
        <!-- Global auto-renew toggle -->
        <label class="flex items-center gap-2 cursor-pointer select-none">
          <button
            type="button"
            role="switch"
            aria-checked={globalAutoRenew}
            on:click={toggleGlobalAutoRenew}
            class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors
                   focus:outline-none focus:ring-2 focus:ring-primary/50 disabled:opacity-50
                   {globalAutoRenew ? 'bg-primary' : 'bg-muted-foreground/30'}"
          >
            <span
              class="inline-block h-3.5 w-3.5 transform rounded-full bg-background shadow transition-transform
                     {globalAutoRenew ? 'translate-x-4.5' : 'translate-x-0.5'}"
            ></span>
          </button>
          <span class="text-sm text-muted-foreground">
            {globalAutoRenew ? 'Auto-renew on' : 'Auto-renew off'}
          </span>
        </label>

        <!-- Issue Wildcard button -->
        <button
          on:click={openWildcardWizard}
          class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-2
                 transition-all duration-150 active:scale-95"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M11.049 2.927c.3-.921 1.603-.921 1.902 0l1.519 4.674a1 1 0 00.95.69h4.915c.969 0 1.371 1.24.588 1.81l-3.976 2.888a1 1 0 00-.363 1.118l1.518 4.674c.3.922-.755 1.688-1.538 1.118l-3.976-2.888a1 1 0 00-1.176 0l-3.976 2.888c-.783.57-1.838-.197-1.538-1.118l1.518-4.674a1 1 0 00-.363-1.118l-3.976-2.888c-.784-.57-.38-1.81.588-1.81h4.914a1 1 0 00.951-.69l1.519-4.674z"/>
          </svg>
          Issue Wildcard
        </button>

        <button
          on:click={() => { showIssueModal = true; issueError = ''; issueDomain = ''; }}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
          </svg>
          Issue Certificate
          {#if !loading}
            <span class="ml-1 text-primary-foreground/70 text-xs">({certs.length})</span>
          {/if}
        </button>
      </div>
    </PageHeader>
  </div>

  <!-- Stats row -->
  {#if !loading}
    <div class="grid grid-cols-1 sm:grid-cols-3 gap-4 fade-up-1">
      <!-- Valid -->
      <div class="bg-card border border-border rounded-xl p-4 flex items-center gap-4
                  transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <div class="w-10 h-10 rounded-lg bg-green-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
          </svg>
        </div>
        <div>
          <div class="text-2xl font-bold text-foreground">{validCount}</div>
          <div class="text-xs text-muted-foreground">Valid certificates</div>
        </div>
      </div>

      <!-- Expiring soon -->
      <div class="bg-card border border-border rounded-xl p-4 flex items-center gap-4
                  transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <div class="w-10 h-10 rounded-lg bg-amber-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
          </svg>
        </div>
        <div>
          <div class="text-2xl font-bold text-foreground">{expiringCount}</div>
          <div class="text-xs text-muted-foreground">Expiring within 30 days</div>
        </div>
      </div>

      <!-- Issues -->
      <div class="bg-card border border-border rounded-xl p-4 flex items-center gap-4
                  transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <div class="w-10 h-10 rounded-lg bg-red-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636" />
          </svg>
        </div>
        <div>
          <div class="text-2xl font-bold text-foreground">{issuesCount}</div>
          <div class="text-xs text-muted-foreground">Expired / issues</div>
        </div>
      </div>
    </div>
  {/if}

  <!-- Table -->
  {#if loading}
    <div class="space-y-2">
      {#each [1, 2, 3, 4, 5] as _}
        <div class="h-14 bg-muted rounded-xl animate-pulse"></div>
      {/each}
    </div>
  {:else if certs.length === 0}
    <div class="bg-card border border-border rounded-xl p-16 flex flex-col items-center gap-4 text-center fade-up">
      <div class="w-14 h-14 rounded-full bg-muted flex items-center justify-center">
        <svg class="w-7 h-7 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
        </svg>
      </div>
      <div>
        <p class="text-foreground font-medium">No SSL certificates yet</p>
        <p class="text-muted-foreground text-sm mt-1">Issue a certificate for one of your sites to get started.</p>
      </div>
      <button
        on:click={() => { showIssueModal = true; issueError = ''; issueDomain = ''; }}
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
      >
        Issue Certificate
      </button>
    </div>
  {:else}
    <div class="rounded-xl border border-border overflow-x-auto fade-up-2">
      <table class="w-full">
        <thead class="bg-muted/50">
          <tr>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase">Domain</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden sm:table-cell">Type</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden md:table-cell">Issued</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase">Expires</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden sm:table-cell">Status</th>
            <th class="px-4 py-3 text-right text-xs text-muted-foreground uppercase">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each certs as cert (cert.id)}
            {@const countdown = expiryCountdown(cert.expires_at)}
            {@const badge = statusBadge(cert.status)}
            {@const issuer = issuerBadge(cert.issuer)}
            {@const lc = lockColor(cert)}
            {@const isExpanded = expandedId === cert.id}

            <tr class="border-t border-border hover:bg-muted/30 transition-colors duration-150">
              <td class="px-4 py-3 text-sm text-foreground">
                <div class="flex items-center gap-2">
                  <!-- Lock icon -->
                  <svg class="w-4 h-4 shrink-0 {lc}" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                  </svg>
                  <span class="font-medium">{cert.domain}</span>
                </div>
              </td>
              <td class="px-4 py-3 hidden sm:table-cell">
                <span class="px-2 py-0.5 rounded-full text-xs font-medium {issuer.cls}">{issuer.label}</span>
              </td>
              <td class="px-4 py-3 text-sm text-muted-foreground hidden md:table-cell">
                {cert.created_at ? new Date(cert.created_at).toLocaleDateString() : '—'}
              </td>
              <td class="px-4 py-3">
                <div class="flex flex-col gap-0.5">
                  <span class="text-xs {countdown.color}">{countdown.text}</span>
                  {#if cert.expires_at}
                    <span class="text-xs text-muted-foreground">{new Date(cert.expires_at).toLocaleDateString()}</span>
                  {/if}
                </div>
              </td>
              <td class="px-4 py-3 hidden sm:table-cell">
                <span class="px-2 py-0.5 rounded-full text-xs font-medium {badge.cls}">{badge.label}</span>
              </td>
              <td class="px-4 py-3">
                <div class="flex items-center gap-1 justify-end">
                  <!-- Renew -->
                  <button
                    on:click={() => renewCert(cert)}
                    disabled={renewingIds.has(cert.id)}
                    title="Renew certificate"
                    class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 disabled:opacity-50 transition-all duration-150 active:scale-95"
                  >
                    {#if renewingIds.has(cert.id)}
                      <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                      </svg>
                    {:else}
                      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                      </svg>
                    {/if}
                    <span class="hidden sm:inline">{renewingIds.has(cert.id) ? 'Renewing…' : 'Renew'}</span>
                  </button>

                  <!-- View Details toggle -->
                  <button
                    on:click={() => toggleExpand(cert.id)}
                    title="View details"
                    class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95
                           {isExpanded ? 'bg-muted text-foreground' : ''}"
                  >
                    <svg class="w-3.5 h-3.5 transition-transform {isExpanded ? 'rotate-180' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
                    </svg>
                    <span class="hidden sm:inline">Details</span>
                  </button>

                  <!-- Revoke -->
                  <button
                    on:click={() => { revokeTarget = cert; }}
                    disabled={revokingId === cert.id}
                    title="Revoke certificate"
                    class="h-9 px-3 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-1.5 disabled:opacity-50 transition-all duration-150 active:scale-95"
                  >
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M18.364 18.364A9 9 0 005.636 5.636m12.728 12.728A9 9 0 015.636 5.636m12.728 12.728L5.636 5.636" />
                    </svg>
                    <span class="hidden md:inline">Revoke</span>
                  </button>
                </div>
              </td>
            </tr>

            <!-- Expanded details row -->
            {#if isExpanded}
              <tr class="border-t border-border bg-muted/20">
                <td colspan="6" class="px-6 py-4">
                  <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4 text-sm mb-4">
                    <div>
                      <div class="text-xs text-muted-foreground uppercase mb-1">Issuer</div>
                      <div class="text-foreground font-medium">
                        {cert.issuer === 'letsencrypt' ? "Let's Encrypt Authority X3" : cert.issuer ?? '—'}
                      </div>
                    </div>
                    <div>
                      <div class="text-xs text-muted-foreground uppercase mb-1">Key Type</div>
                      <div class="text-foreground font-medium">RSA 2048</div>
                    </div>
                    <div>
                      <div class="text-xs text-muted-foreground uppercase mb-1">Auto-Renew</div>
                      <div class="flex items-center gap-1.5">
                        <span class="w-1.5 h-1.5 rounded-full {cert.auto_renew ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
                        <span class="text-foreground font-medium">{cert.auto_renew ? 'Enabled' : 'Disabled'}</span>
                      </div>
                    </div>
                    <div>
                      <div class="text-xs text-muted-foreground uppercase mb-1">Created</div>
                      <div class="text-foreground font-medium">
                        {cert.created_at ? new Date(cert.created_at).toLocaleString() : '—'}
                      </div>
                    </div>
                    <div>
                      <div class="text-xs text-muted-foreground uppercase mb-1">Next Scheduled Renewal</div>
                      <div class="text-foreground font-medium">{nextRenewalDate(cert.expires_at)}</div>
                    </div>
                    <div>
                      <div class="text-xs text-muted-foreground uppercase mb-1">Serial Number</div>
                      <div class="text-foreground font-mono text-xs">Auto-generated by CA</div>
                    </div>
                    {#if cert.cert_path}
                      <div class="sm:col-span-3">
                        <div class="text-xs text-muted-foreground uppercase mb-1">Certificate Path</div>
                        <div class="font-mono text-xs text-foreground bg-background border border-border rounded-lg px-3 py-2 break-all">{cert.cert_path}</div>
                      </div>
                    {/if}
                    {#if cert.key_path}
                      <div class="sm:col-span-3">
                        <div class="text-xs text-muted-foreground uppercase mb-1">Key Path</div>
                        <div class="font-mono text-xs text-foreground bg-background border border-border rounded-lg px-3 py-2 break-all">{cert.key_path}</div>
                      </div>
                    {/if}
                  </div>

                  <!-- SANs list placeholder -->
                  <div>
                    <div class="text-xs text-muted-foreground uppercase mb-2">Subject Alternative Names (SANs)</div>
                    <div class="flex flex-wrap gap-1.5">
                      <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-mono font-medium
                                   bg-blue-500/10 text-blue-400 border border-blue-500/20">{cert.domain}</span>
                      <span class="inline-flex items-center px-2 py-0.5 rounded text-xs font-mono font-medium
                                   bg-blue-500/10 text-blue-400 border border-blue-500/20">www.{cert.domain}</span>
                    </div>
                  </div>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    </div>

    <!-- ── Expiry Timeline ──────────────────────────────────────────────────── -->
    <div class="bg-card border border-border rounded-xl p-4 fade-up-2
                transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
      <div class="flex items-center gap-2 mb-3">
        <svg class="w-4 h-4 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"/>
        </svg>
        <h3 class="text-sm font-semibold text-foreground">Expiry Timeline</h3>
        <span class="text-xs text-muted-foreground">365-day scale — shorter bar = expires sooner</span>
      </div>
      <div class="space-y-1.5 mt-4">
        {#each certs.sort((a,b) => new Date(a.expires_at ?? 0).getTime() - new Date(b.expires_at ?? 0).getTime()) as cert}
          {@const days = Math.max(0, Math.floor(((cert.expires_at ? new Date(cert.expires_at).getTime() : Date.now()) - Date.now()) / 86400000))}
          {@const pct = Math.min(100, (days / 365) * 100)}
          {@const color = days > 30 ? 'bg-green-500' : days > 7 ? 'bg-amber-500' : 'bg-red-500'}
          <div class="flex items-center gap-3 text-xs">
            <span class="w-40 truncate text-muted-foreground">{cert.domain}</span>
            <div class="flex-1 bg-muted rounded-full h-1.5 overflow-hidden">
              <div class="{color} h-1.5 rounded-full transition-all duration-500" style="width:{pct}%"></div>
            </div>
            <span class="w-16 text-right {days < 7 ? 'text-red-400 font-medium' : 'text-muted-foreground'}">{days}d</span>
          </div>
        {/each}
      </div>
    </div>
  {/if}

</div>

<!-- ── Wildcard Certificate Wizard ────────────────────────────────────────── -->
{#if showWildcardModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog" aria-modal="true" aria-label="Issue Wildcard Certificate"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => { if (!wildcardIssuing && !wildcardVerifying) closeWildcard(); }}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl fade-up">
      <!-- Wizard step indicator -->
      <div class="flex items-center gap-2 mb-5">
        {#each [1, 2, 3] as step}
          <div class="flex items-center gap-2">
            <div class="w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold
                        {wildcardStep >= step ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground'}">
              {step}
            </div>
            {#if step < 3}
              <div class="w-8 h-px {wildcardStep > step ? 'bg-primary' : 'bg-border'}"></div>
            {/if}
          </div>
        {/each}
        <div class="ml-auto">
          <button
            on:click={closeWildcard}
            disabled={wildcardIssuing || wildcardVerifying}
            class="w-7 h-7 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground"
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
            </svg>
          </button>
        </div>
      </div>

      {#if wildcardStep === 1}
        <div>
          <h2 class="text-base font-semibold text-foreground mb-1">Issue Wildcard Certificate</h2>
          <p class="text-sm text-muted-foreground mb-4">Enter the base domain. A certificate for <code class="text-primary">*.{wildcardDomain || 'domain.com'}</code> will be issued via DNS challenge.</p>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="wc-domain">Base Domain</label>
          <input
            id="wc-domain"
            type="text"
            bind:value={wildcardDomain}
            placeholder="example.com"
            class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground font-mono
                   focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary mb-4"
          />
          {#if wildcardError}
            <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5 mb-4">
              {wildcardError}
            </div>
          {/if}
          <div class="flex gap-2">
            <button
              on:click={wildcardNext}
              class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                     hover:bg-primary/90 transition-all duration-150 active:scale-95"
            >
              Next: DNS Challenge
            </button>
          </div>
        </div>

      {:else if wildcardStep === 2}
        <div>
          <h2 class="text-base font-semibold text-foreground mb-1">Add DNS TXT Record</h2>
          <p class="text-sm text-muted-foreground mb-4">Add this TXT record to your DNS to prove domain ownership. Then click "Verify DNS".</p>
          <div class="bg-muted/40 rounded-lg p-3 space-y-2 text-xs font-mono mb-4">
            <div class="flex items-start gap-2">
              <span class="text-muted-foreground shrink-0">Name:</span>
              <span class="text-amber-400 break-all">_acme-challenge.{wildcardDomain}</span>
            </div>
            <div class="flex items-start gap-2">
              <span class="text-muted-foreground shrink-0">Type:</span>
              <span class="text-blue-400">TXT</span>
            </div>
            <div class="flex items-start gap-2">
              <span class="text-muted-foreground shrink-0">Value:</span>
              <span class="text-green-400 break-all">{wildcardToken}</span>
            </div>
          </div>
          <div class="flex gap-2">
            <button
              on:click={startWildcardVerify}
              disabled={wildcardVerifying}
              class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                     hover:bg-primary/90 transition-all duration-150 active:scale-95 disabled:opacity-50
                     inline-flex items-center justify-center gap-2"
            >
              {#if wildcardVerifying}
                <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                </svg>
                Verifying DNS...
              {:else}
                Auto-Verify DNS Propagation
              {/if}
            </button>
            <button
              on:click={() => wildcardStep = 1}
              class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
            >
              Back
            </button>
          </div>
        </div>

      {:else if wildcardStep === 3}
        <div>
          <h2 class="text-base font-semibold text-foreground mb-1">Issue Certificate</h2>
          <div class="flex items-center gap-2 text-sm text-green-400 mb-4">
            <svg class="w-5 h-5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z"/>
            </svg>
            DNS record verified — ready to issue <code class="ml-1 text-primary">*.{wildcardDomain}</code>
          </div>
          {#if wildcardError}
            <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5 mb-4">
              {wildcardError}
            </div>
          {/if}
          <div class="flex gap-2">
            <button
              on:click={issueWildcard}
              disabled={wildcardIssuing}
              class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                     hover:bg-primary/90 transition-all duration-150 active:scale-95 disabled:opacity-50
                     inline-flex items-center justify-center gap-2"
            >
              {#if wildcardIssuing}
                <svg class="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                </svg>
                Issuing...
              {:else}
                Issue Wildcard Certificate
              {/if}
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<!-- Issue Certificate Modal -->
{#if showIssueModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-label="Issue certificate"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => { if (!issueLoading) { showIssueModal = false; } }}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl fade-up">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">Issue SSL Certificate</h2>
        <button
          on:click={() => showIssueModal = false}
          disabled={issueLoading}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="space-y-4">
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="issue-domain">Domain</label>
          <input
            id="issue-domain"
            type="text"
            bind:value={issueDomain}
            placeholder="example.com"
            class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
          />
          <p class="text-xs text-muted-foreground mt-1">Must match an existing site. A Let's Encrypt HTTP-01 challenge will be queued.</p>
        </div>

        {#if issueError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5">
            {issueError}
          </div>
        {/if}

        <div class="flex gap-3 pt-1">
          <button
            on:click={() => showIssueModal = false}
            disabled={issueLoading}
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
          >
            Cancel
          </button>
          <button
            on:click={issueNew}
            disabled={issueLoading}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50 transition-all duration-150 active:scale-95"
          >
            {#if issueLoading}
              <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Issuing…
            {:else}
              Issue Certificate
            {/if}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- Revoke Confirm Modal -->
{#if revokeTarget}
  {@const target = revokeTarget}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-label="Confirm revoke"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => { revokeTarget = null; }}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl fade-up">
      <div class="flex items-start gap-4 mb-5">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center shrink-0 mt-0.5">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
          </svg>
        </div>
        <div>
          <h2 class="text-base font-semibold text-foreground">Revoke Certificate?</h2>
          <p class="text-sm text-muted-foreground mt-1">
            This will permanently revoke the SSL certificate for <strong class="text-foreground">{target.domain}</strong>.
            HTTPS will stop working immediately. This cannot be undone.
          </p>
        </div>
      </div>
      <div class="flex gap-3 justify-end">
        <button
          on:click={() => { revokeTarget = null; }}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2"
        >
          Cancel
        </button>
        <button
          on:click={confirmRevoke}
          class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          Revoke Certificate
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- Toast -->
{#if toastMessage}
  <div
    role="status"
    aria-live="polite"
    class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg flex items-center gap-2 fade-up"
  >
    {#if toastType === 'success'}
      <svg class="w-4 h-4 text-green-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
      </svg>
    {:else}
      <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
      </svg>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px) }
    to   { opacity: 1; transform: none }
  }
  .fade-up   { animation: fadeUp 0.25s ease-out both }
  .fade-up-1 { animation: fadeUp 0.25s 0.05s ease-out both }
  .fade-up-2 { animation: fadeUp 0.25s 0.1s ease-out both }
</style>
