<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import type { EmailDomain, EmailAccount, SmtpLimitsResponse, SmtpLimitsRequest } from '$api/client';

  // ── Tab state ──────────────────────────────────────────────────────────────
  type Tab = 'accounts' | 'forwarders' | 'client-setup' | 'smtp-limits';
  let activeTab: Tab = 'accounts';

  // ── Shared state ──────────────────────────────────────────────────────────
  let domains: EmailDomain[] = [];
  let accounts: EmailAccount[] = [];
  let loading = true;
  let loadError = '';

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';

  // ── Accounts tab state ────────────────────────────────────────────────────
  let searchQuery = '';
  let domainFilter = '';
  let statusFilter = 'all';
  let showServerInfo = false;

  // Create email modal
  let showCreateModal = false;
  let createUsername = '';
  let createDomainId = '';
  let createPassword = '';
  let createQuotaMb = 1024;
  let createLoading = false;
  let createError = '';
  let showPassword = false;

  // Change password inline state: accountId → new password string
  let changePwMap: Record<string, string> = {};
  let changePwLoading: Record<string, boolean> = {};

  // Edit quota inline state: accountId → new quota number
  let editQuotaMap: Record<string, number> = {};
  let editQuotaLoading: Record<string, boolean> = {};

  // Delete confirmation modal
  let deleteTarget: EmailAccount | null = null;
  let deleteConfirmCheck = false;
  let deleteLoading = false;

  // ── SMTP Limits tab state ─────────────────────────────────────────────────
  let smtpLimits: SmtpLimitsResponse[] = [];
  let smtpLoading = false;
  let smtpLoadError = '';

  // Edit limits modal
  let showEditModal = false;
  let editDomain = '';
  let editForm: SmtpLimitsRequest = {
    hourly_limit: 500,
    daily_limit: 5000,
    rate_action: 'queue',
    relay_enabled: true,
    require_spf: false,
    require_dkim: false,
    enabled: true,
  };
  let editLoading = false;
  let editError = '';

  // Reset counter loading per domain
  let resetLoading: Record<string, boolean> = {};

  // Global defaults collapsible
  let showGlobalDefaults = false;
  let globalDefaults = { hourly_limit: 500, daily_limit: 5000, rate_action: 'queue' as 'queue' | 'reject' | 'bounce' };
  let globalDefaultsLoading = false;

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    if (typeof location !== "undefined" && new URLSearchParams(location.search).get("new") === "1") showCreateModal = true;
    await loadData();
  });

  async function loadData() {
    loading = true;
    loadError = '';
    try {
      [domains, accounts] = await Promise.all([
        api.email.domains.list(),
        api.email.accounts.list(),
      ]);
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load email data';
    } finally {
      loading = false;
    }
  }

  async function loadSmtpLimits() {
    smtpLoading = true;
    smtpLoadError = '';
    try {
      smtpLimits = await api.email.smtpLimits.list();
    } catch (err: unknown) {
      smtpLoadError = (err as { message?: string })?.message ?? 'Failed to load SMTP limits';
    } finally {
      smtpLoading = false;
    }
  }

  function switchTab(tab: Tab) {
    activeTab = tab;
    if (tab === 'smtp-limits' && smtpLimits.length === 0 && !smtpLoading) {
      void loadSmtpLimits();
    }
  }

  // ── Derived / filtered ─────────────────────────────────────────────────────

  $: domainMap = domains.reduce((acc, d) => {
    acc[d.id] = d.domain;
    return acc;
  }, {} as Record<string, string>);

  function accountDomain(acct: EmailAccount): string {
    if (acct.address) return acct.address.split('@')[1] ?? '';
    return domainMap[acct.domain_id ?? ''] ?? '';
  }

  $: filteredAccounts = accounts.filter(acct => {
    const addr = acct.address ?? `${acct.local_part}@${accountDomain(acct)}`;
    const matchSearch  = !searchQuery  || addr.toLowerCase().includes(searchQuery.toLowerCase());
    const matchDomain  = !domainFilter || accountDomain(acct) === domainFilter;
    const matchStatus  = statusFilter === 'all'
      || acct.status === statusFilter
      || (statusFilter === 'active' && acct.status === 'provisioning');
    return matchSearch && matchDomain && matchStatus;
  });

  $: totalAccounts    = accounts.length;
  $: totalStorageMb   = accounts.reduce((sum, a) => sum + (a.used_mb ?? 0), 0);
  $: domainsWithEmail = new Set(accounts.map(a => accountDomain(a))).size;
  $: accountDomainList = [...new Set(accounts.map(a => accountDomain(a)))].filter(Boolean).sort();

  // ── Helpers ────────────────────────────────────────────────────────────────
  function quotaPercent(used: number, quota: number): number {
    if (!quota) return 0;
    return Math.min((used / quota) * 100, 100);
  }

  function formatMb(mb: number): string {
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${mb} MB`;
  }

  function passwordStrength(pw: string): { score: number; label: string; color: string } {
    let score = 0;
    if (pw.length >= 10) score++;
    if (pw.length >= 14) score++;
    if (/[A-Z]/.test(pw)) score++;
    if (/[a-z]/.test(pw)) score++;
    if (/[0-9]/.test(pw)) score++;
    if (/[^a-zA-Z0-9]/.test(pw)) score++;
    if (score <= 2) return { score, label: 'Weak',   color: 'bg-red-500' };
    if (score <= 4) return { score, label: 'Fair',   color: 'bg-amber-500' };
    return              { score, label: 'Strong', color: 'bg-green-500' };
  }

  function generatePassword(): string {
    const upper   = 'ABCDEFGHJKLMNPQRSTUVWXYZ';
    const lower   = 'abcdefghjkmnpqrstuvwxyz';
    const digits  = '23456789';
    const special = '!@#$%^&*';
    const all = upper + lower + digits + special;
    let pw = [
      upper[Math.floor(Math.random() * upper.length)],
      lower[Math.floor(Math.random() * lower.length)],
      digits[Math.floor(Math.random() * digits.length)],
      special[Math.floor(Math.random() * special.length)],
    ];
    for (let i = 0; i < 10; i++) pw.push(all[Math.floor(Math.random() * all.length)]);
    return pw.sort(() => Math.random() - 0.5).join('');
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType    = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function limitPct(used: number, limit: number): number {
    if (!limit) return 0;
    return Math.min((used / limit) * 100, 100);
  }

  function limitColor(pct: number): string {
    if (pct >= 80) return 'bg-red-500';
    if (pct >= 50) return 'bg-amber-500';
    return 'bg-green-500';
  }

  function rateActionBadge(action: string): string {
    if (action === 'reject') return 'bg-red-500/10 text-red-400 border-red-500/20';
    if (action === 'bounce') return 'bg-amber-500/10 text-amber-400 border-amber-500/20';
    return 'bg-blue-500/10 text-blue-400 border-blue-500/20';
  }

  // ── Input validation ──────────────────────────────────────────────────────

  function validateUsername(v: string): string {
    if (!v) return 'Username is required';
    if (!/^[a-z][a-z0-9_.+\-]*$/.test(v)) return 'Username may only contain lowercase letters, numbers, dots, underscores, plus or hyphens';
    if (v.length > 64) return 'Username too long (max 64)';
    return '';
  }

  function validatePassword(v: string): string {
    if (!v) return 'Password is required';
    if (v.length < 10) return 'Minimum 10 characters';
    if (!/[A-Z]/.test(v)) return 'Must contain an uppercase letter';
    if (!/[a-z]/.test(v)) return 'Must contain a lowercase letter';
    if (!/[0-9]/.test(v)) return 'Must contain a number';
    return '';
  }

  // ── API actions — Accounts ─────────────────────────────────────────────────

  async function handleCreate(e: SubmitEvent) {
    e.preventDefault();
    createError   = '';

    // Client-side validation
    const usernameErr = validateUsername(createUsername.toLowerCase().trim());
    if (usernameErr) { createError = usernameErr; return; }
    const passwordErr = validatePassword(createPassword);
    if (passwordErr) { createError = passwordErr; return; }
    if (!createDomainId) { createError = 'Please select a domain'; return; }

    createLoading = true;
    try {
      const account = await api.email.accounts.create({
        domain_id:  createDomainId,
        local_part: createUsername.toLowerCase().trim(),
        password:   createPassword,
        quota_mb:   createQuotaMb,
      });
      accounts      = [...accounts, account];
      showCreateModal = false;
      createUsername  = '';
      createPassword  = '';
      createQuotaMb   = 1024;
      showToast('Email account created', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      createError = e.message ?? 'Failed to create account';
    } finally {
      createLoading = false;
    }
  }

  async function loginWebmail(acct: EmailAccount) {
    try {
      const { sso_url } = await api.email.webmailSso(acct.id);
      window.open(sso_url, '_blank');
    } catch {
      showToast('Webmail login failed', 'error');
    }
  }

  async function handleChangePassword(acct: EmailAccount) {
    const newPw = changePwMap[acct.id] ?? '';
    if (!newPw) return;
    changePwLoading = { ...changePwLoading, [acct.id]: true };
    try {
      await api.email.accounts.changePassword(acct.id, newPw);
      changePwMap = { ...changePwMap, [acct.id]: '' };
      showToast('Password updated', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to update password', 'error');
    } finally {
      changePwLoading = { ...changePwLoading, [acct.id]: false };
    }
  }

  async function handleEditQuota(acct: EmailAccount) {
    const newQuota = editQuotaMap[acct.id];
    if (newQuota == null || newQuota < 1) return;
    editQuotaLoading = { ...editQuotaLoading, [acct.id]: true };
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch(`/api/v1/email/accounts/${acct.id}`, {
        method: 'PUT',
        headers: {
          'Content-Type': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
        body: JSON.stringify({ quota_mb: newQuota }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      accounts = accounts.map(a => a.id === acct.id ? { ...a, quota_mb: newQuota } : a);
      const m = { ...editQuotaMap };
      delete m[acct.id];
      editQuotaMap = m;
      showToast('Quota updated', 'success');
    } catch {
      showToast('Failed to update quota', 'error');
    } finally {
      editQuotaLoading = { ...editQuotaLoading, [acct.id]: false };
    }
  }

  function openDeleteModal(acct: EmailAccount) {
    deleteTarget       = acct;
    deleteConfirmCheck = false;
  }

  async function handleDelete() {
    if (!deleteTarget || !deleteConfirmCheck) return;
    deleteLoading = true;
    try {
      await api.email.accounts.delete(deleteTarget.id);
      accounts     = accounts.filter(a => a.id !== deleteTarget!.id);
      deleteTarget = null;
      showToast('Account deleted', 'success');
    } catch {
      showToast('Failed to delete account', 'error');
    } finally {
      deleteLoading = false;
    }
  }

  // ── API actions — SMTP Limits ──────────────────────────────────────────────

  function openEditModal(limit: SmtpLimitsResponse) {
    editDomain = limit.domain;
    editForm = {
      hourly_limit:  limit.hourly_limit,
      daily_limit:   limit.daily_limit,
      rate_action:   limit.rate_action,
      relay_enabled: limit.relay_enabled,
      require_spf:   limit.require_spf,
      require_dkim:  limit.require_dkim,
      enabled:       limit.enabled,
    };
    editError    = '';
    showEditModal = true;
  }

  async function handleSaveLimits() {
    editError = '';
    // Client-side validation
    const h = editForm.hourly_limit ?? 0;
    const d = editForm.daily_limit ?? 0;
    if (h !== 0 && (h < 1 || h > 100000)) {
      editError = 'Hourly limit must be between 1 and 100,000 (or 0 for unlimited)';
      return;
    }
    if (d !== 0 && (d < 1 || d > 1000000)) {
      editError = 'Daily limit must be between 1 and 1,000,000 (or 0 for unlimited)';
      return;
    }
    if (h !== 0 && d !== 0 && d < h) {
      editError = 'Daily limit must be greater than or equal to the hourly limit';
      return;
    }
    if (!['queue', 'reject', 'bounce'].includes(editForm.rate_action ?? '')) {
      editError = 'Rate action must be queue, reject, or bounce';
      return;
    }
    editLoading = true;
    try {
      const updated = await api.email.smtpLimits.upsert(editDomain, editForm);
      smtpLimits = smtpLimits.map(l => l.domain === editDomain ? updated : l);
      showEditModal = false;
      showToast('SMTP limits saved', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      editError = e.message ?? 'Failed to save limits';
    } finally {
      editLoading = false;
    }
  }

  async function handleResetCounters(domain: string) {
    resetLoading = { ...resetLoading, [domain]: true };
    try {
      await api.email.smtpLimits.resetCounters(domain);
      smtpLimits = smtpLimits.map(l =>
        l.domain === domain ? { ...l, sent_this_hour: 0, sent_today: 0 } : l
      );
      showToast(`Counters reset for ${domain}`, 'success');
    } catch {
      showToast('Failed to reset counters', 'error');
    } finally {
      resetLoading = { ...resetLoading, [domain]: false };
    }
  }

  async function handleToggleEnabled(limit: SmtpLimitsResponse) {
    try {
      const updated = await api.email.smtpLimits.upsert(limit.domain, { enabled: !limit.enabled });
      smtpLimits = smtpLimits.map(l => l.domain === limit.domain ? updated : l);
      showToast(`${limit.domain} ${updated.enabled ? 'enabled' : 'disabled'}`, 'success');
    } catch {
      showToast('Failed to update', 'error');
    }
  }

  async function handleToggleRelay(limit: SmtpLimitsResponse) {
    try {
      const updated = await api.email.smtpLimits.upsert(limit.domain, { relay_enabled: !limit.relay_enabled });
      smtpLimits = smtpLimits.map(l => l.domain === limit.domain ? updated : l);
    } catch {
      showToast('Failed to update relay setting', 'error');
    }
  }

  function setPreset(field: 'hourly_limit' | 'daily_limit', value: number) {
    editForm = { ...editForm, [field]: value };
  }

  // ── Computed helpers ───────────────────────────────────────────────────────
  $: pwStrength = passwordStrength(createPassword);
  $: selectedDomainName = domains.find(d => d.id === createDomainId)?.domain ?? 'domain.com';
</script>

<svelte:head>
  <title>Email — OrbitCP</title>
</svelte:head>

<div class="p-4 lg:p-6 space-y-5">

  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
      <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round"
          d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
      </svg>
      <span>{loadError}</span>
      <button on:click={() => void loadData()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- ── Header ────────────────────────────────────────────────────────────── -->
  <div class="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
    <div class="flex items-center gap-3">
      <div>
        <h1 class="text-xl font-semibold text-foreground">Email</h1>
        <p class="text-sm text-muted-foreground mt-0.5">Manage mailboxes, forwarders and sending limits</p>
      </div>
      {#if !loading}
        <span class="inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium
                     bg-primary/10 text-primary border border-primary/20">
          {totalAccounts} accounts
        </span>
      {/if}
    </div>
    {#if activeTab === 'accounts'}
      <button
        on:click={() => { createDomainId = domains[0]?.id ?? ''; showCreateModal = true; }}
        disabled={loading || domains.length === 0}
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
               hover:bg-primary/90 inline-flex items-center gap-2 transition-colors disabled:opacity-50"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
        </svg>
        Create Email Account
      </button>
    {/if}
  </div>

  <!-- ── Sub-tabs ───────────────────────────────────────────────────────────── -->
  <div class="flex gap-1 border-b border-border">
    {#each [
      { id: 'accounts',     label: 'Accounts' },
      { id: 'forwarders',   label: 'Forwarders' },
      { id: 'client-setup', label: 'Client Setup' },
      { id: 'smtp-limits',  label: 'SMTP Limits' },
    ] as tab}
      <button
        on:click={() => switchTab(tab.id as Tab)}
        class="px-4 py-2 text-sm font-medium transition-all duration-200 border-b-2 -mb-px
               {activeTab === tab.id
                 ? 'border-primary text-foreground'
                 : 'border-transparent text-muted-foreground hover:text-foreground hover:border-border'}"
      >{tab.label}</button>
    {/each}
  </div>

  <!-- ═══════════════════════════════════════════════════════════════════════ -->
  <!-- TAB: Accounts                                                          -->
  <!-- ═══════════════════════════════════════════════════════════════════════ -->
  {#if activeTab === 'accounts'}
    <div class="transition-all duration-200 space-y-5">

      <!-- Stats row -->
      {#if loading}
        <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
          {#each [1,2,3] as _}
            <div class="h-20 bg-muted rounded-xl animate-pulse"></div>
          {/each}
        </div>
      {:else}
        <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
          <div class="bg-card border border-border rounded-xl px-4 py-3 flex items-center gap-3
                      hover:shadow-lg hover:-translate-y-0.5 transition-all duration-200">
            <div class="w-9 h-9 rounded-lg bg-blue-500/10 flex items-center justify-center shrink-0">
              <svg class="w-5 h-5 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/>
              </svg>
            </div>
            <div>
              <p class="text-2xl font-semibold text-foreground">{totalAccounts}</p>
              <p class="text-xs text-muted-foreground">Total accounts</p>
            </div>
          </div>

          <div class="bg-card border border-border rounded-xl px-4 py-3 flex items-center gap-3
                      hover:shadow-lg hover:-translate-y-0.5 transition-all duration-200">
            <div class="w-9 h-9 rounded-lg bg-purple-500/10 flex items-center justify-center shrink-0">
              <svg class="w-5 h-5 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"/>
              </svg>
            </div>
            <div>
              <p class="text-2xl font-semibold text-foreground">{formatMb(totalStorageMb)}</p>
              <p class="text-xs text-muted-foreground">Total storage used</p>
            </div>
          </div>

          <div class="bg-card border border-border rounded-xl px-4 py-3 flex items-center gap-3
                      hover:shadow-lg hover:-translate-y-0.5 transition-all duration-200">
            <div class="w-9 h-9 rounded-lg bg-green-500/10 flex items-center justify-center shrink-0">
              <svg class="w-5 h-5 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9"/>
              </svg>
            </div>
            <div>
              <p class="text-2xl font-semibold text-foreground">{domainsWithEmail}</p>
              <p class="text-xs text-muted-foreground">Domains with email</p>
            </div>
          </div>
        </div>
      {/if}

      <!-- Filter + Search -->
      <div class="flex flex-col sm:flex-row gap-2">
        <div class="relative flex-1 max-w-xs">
          <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground"
               fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"/>
          </svg>
          <input type="search" bind:value={searchQuery} placeholder="Search email addresses..."
                 class="w-full h-9 pl-9 pr-3 rounded-lg border border-border bg-background text-sm
                        text-foreground placeholder:text-muted-foreground
                        focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"/>
        </div>

        <select bind:value={domainFilter}
                class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                       focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
          <option value="">All domains</option>
          {#each accountDomainList as d}
            <option value={d}>@{d}</option>
          {/each}
        </select>

        <div class="flex rounded-lg border border-border overflow-hidden text-sm h-9">
          {#each ['all', 'active', 'suspended'] as s}
            <button
              on:click={() => statusFilter = s}
              class="px-3 h-full transition-colors
                     {statusFilter === s
                       ? 'bg-primary text-primary-foreground font-medium'
                       : 'bg-background text-muted-foreground hover:bg-muted hover:text-foreground'}"
            >{s === 'all' ? 'All' : s === 'active' ? 'Active' : 'Suspended'}</button>
          {/each}
        </div>
      </div>

      <!-- Accounts table -->
      {#if loading}
        <div class="bg-card border border-border rounded-xl overflow-hidden">
          <div class="px-4 py-3 border-b border-border bg-muted/50">
            <div class="h-4 w-48 bg-muted rounded animate-pulse"></div>
          </div>
          {#each [1,2,3,4] as _}
            <div class="px-4 py-4 border-b border-border last:border-0 flex gap-4">
              <div class="h-4 w-48 bg-muted rounded animate-pulse"></div>
              <div class="h-4 w-24 bg-muted rounded animate-pulse"></div>
              <div class="h-4 w-32 bg-muted rounded animate-pulse"></div>
            </div>
          {/each}
        </div>
      {:else if filteredAccounts.length === 0}
        <div class="bg-card border border-border rounded-xl p-12 text-center">
          <div class="w-12 h-12 rounded-full bg-muted/50 flex items-center justify-center mx-auto mb-3">
            <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/>
            </svg>
          </div>
          <p class="text-sm font-medium text-foreground mb-1">
            {searchQuery || domainFilter || statusFilter !== 'all' ? 'No accounts match your filters' : 'No email accounts yet'}
          </p>
          <p class="text-xs text-muted-foreground mb-4">
            {searchQuery || domainFilter || statusFilter !== 'all' ? 'Try adjusting your search or filters.' : 'Create your first email account to get started.'}
          </p>
          {#if !searchQuery && !domainFilter && statusFilter === 'all' && domains.length > 0}
            <button
              on:click={() => { createDomainId = domains[0]?.id ?? ''; showCreateModal = true; }}
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                     hover:bg-primary/90 inline-flex items-center gap-2 transition-colors"
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4"/>
              </svg>
              Create Email Account
            </button>
          {/if}
        </div>
      {:else}
        <div class="rounded-xl border border-border overflow-x-auto">
          <table class="w-full text-sm">
            <thead class="bg-muted/50">
              <tr>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider">Email Address</th>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider hidden md:table-cell">Domain</th>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider hidden sm:table-cell">Quota</th>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider">Status</th>
                <th class="px-4 py-3 text-right text-xs text-muted-foreground uppercase tracking-wider">Actions</th>
              </tr>
            </thead>
            <tbody>
              {#each filteredAccounts as acct}
                {@const addr    = acct.address ?? `${acct.local_part ?? ''}@${accountDomain(acct)}`}
                {@const domain  = accountDomain(acct)}
                {@const pct     = quotaPercent(acct.used_mb, acct.quota_mb)}
                <tr class="border-b border-border last:border-0 hover:bg-muted/30 transition-colors duration-150">
                  <!-- Email address -->
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-2">
                      <div class="w-7 h-7 rounded-full bg-primary/10 flex items-center justify-center shrink-0">
                        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M16 12a4 4 0 10-8 0 4 4 0 008 0zm0 0v1.5a2.5 2.5 0 005 0V12a9 9 0 10-9 9m4.5-1.206a8.959 8.959 0 01-4.5 1.207"/>
                        </svg>
                      </div>
                      <span class="font-mono text-foreground text-sm">{addr}</span>
                    </div>

                    {#if changePwMap[acct.id] !== undefined}
                      <div class="mt-2 flex items-center gap-1.5">
                        <input
                          type="password"
                          placeholder="New password (min 10 chars)"
                          bind:value={changePwMap[acct.id]}
                          class="h-8 flex-1 rounded-lg border border-border bg-background px-2 text-xs
                                 text-foreground placeholder:text-muted-foreground
                                 focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                        />
                        <button
                          on:click={() => handleChangePassword(acct)}
                          disabled={changePwLoading[acct.id]}
                          class="h-8 px-2 rounded-lg bg-primary text-primary-foreground text-xs font-medium
                                 hover:bg-primary/90 transition-colors disabled:opacity-50"
                        >Save</button>
                        <button
                          on:click={() => { const m = {...changePwMap}; delete m[acct.id]; changePwMap = m; }}
                          class="h-8 px-2 rounded-lg border border-border text-xs text-muted-foreground
                                 hover:bg-muted transition-colors"
                        >Cancel</button>
                      </div>
                    {/if}

                    {#if editQuotaMap[acct.id] !== undefined && editQuotaMap[acct.id] > 0}
                      <div class="mt-2 flex items-center gap-1.5">
                        <input
                          type="number"
                          min="100" max="10240" step="100"
                          bind:value={editQuotaMap[acct.id]}
                          class="h-8 w-28 rounded-lg border border-border bg-background px-2 text-xs
                                 text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                        />
                        <span class="text-xs text-muted-foreground">MB</span>
                        <button
                          on:click={() => handleEditQuota(acct)}
                          disabled={editQuotaLoading[acct.id]}
                          class="h-8 px-2 rounded-lg bg-primary text-primary-foreground text-xs font-medium
                                 hover:bg-primary/90 transition-colors disabled:opacity-50"
                        >Save</button>
                        <button
                          on:click={() => { const m = {...editQuotaMap}; delete m[acct.id]; editQuotaMap = m; }}
                          class="h-8 px-2 rounded-lg border border-border text-xs text-muted-foreground
                                 hover:bg-muted transition-colors"
                        >Cancel</button>
                      </div>
                    {/if}
                  </td>

                  <!-- Domain -->
                  <td class="px-4 py-3 hidden md:table-cell">
                    <span class="text-muted-foreground text-xs font-mono">@{domain}</span>
                  </td>

                  <!-- Quota with progress bar -->
                  <td class="px-4 py-3 hidden sm:table-cell">
                    <div class="space-y-1">
                      <div class="flex items-center gap-2">
                        <div class="h-1.5 w-20 bg-muted rounded-full overflow-hidden">
                          <div class="h-full rounded-full transition-all
                                      {pct > 90 ? 'bg-red-500' : pct > 70 ? 'bg-amber-500' : 'bg-primary'}"
                               style="width: {pct}%"></div>
                        </div>
                        <span class="text-xs text-muted-foreground whitespace-nowrap">
                          {formatMb(acct.used_mb)} / {formatMb(acct.quota_mb)}
                        </span>
                      </div>
                    </div>
                  </td>

                  <!-- Status badge -->
                  <td class="px-4 py-3">
                    {#if acct.status === 'active'}
                      <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium
                                   bg-green-500/10 text-green-400 border border-green-500/20">Active</span>
                    {:else if acct.status === 'suspended'}
                      <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium
                                   bg-red-500/10 text-red-400 border border-red-500/20">Suspended</span>
                    {:else if acct.status === 'provisioning'}
                      <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium
                                   bg-amber-500/10 text-amber-400 border border-amber-500/20">Provisioning</span>
                    {:else}
                      <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium
                                   bg-muted text-muted-foreground border border-border">{acct.status}</span>
                    {/if}
                  </td>

                  <!-- Actions -->
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-1 justify-end">
                      <button
                        on:click={() => loginWebmail(acct)}
                        title="Open Webmail"
                        class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground
                               hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-colors"
                      >
                        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/>
                        </svg>
                        <span class="hidden sm:inline">Webmail</span>
                      </button>

                      <button
                        on:click={() => {
                          if (changePwMap[acct.id] !== undefined) {
                            const m = {...changePwMap}; delete m[acct.id]; changePwMap = m;
                          } else {
                            changePwMap = { ...changePwMap, [acct.id]: '' };
                          }
                        }}
                        title="Change Password"
                        class="h-9 w-9 rounded-lg border border-border flex items-center justify-center
                               text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
                      >
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"/>
                        </svg>
                      </button>

                      <button
                        on:click={() => {
                          if (editQuotaMap[acct.id] !== undefined) {
                            const m = {...editQuotaMap}; delete m[acct.id]; editQuotaMap = m;
                          } else {
                            editQuotaMap = { ...editQuotaMap, [acct.id]: acct.quota_mb };
                          }
                        }}
                        title="Edit Quota"
                        class="h-9 w-9 rounded-lg border border-border flex items-center justify-center
                               text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
                      >
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4"/>
                        </svg>
                      </button>

                      <button
                        on:click={() => openDeleteModal(acct)}
                        title="Delete account"
                        class="h-9 w-9 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20
                               flex items-center justify-center hover:bg-red-500/20 transition-colors"
                      >
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
                        </svg>
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}

    </div>
  {/if}

  <!-- ═══════════════════════════════════════════════════════════════════════ -->
  <!-- TAB: Forwarders                                                        -->
  <!-- ═══════════════════════════════════════════════════════════════════════ -->
  {#if activeTab === 'forwarders'}
    <div class="transition-all duration-200">
      <div class="bg-card border border-border rounded-xl p-8 text-center text-sm text-muted-foreground">
        <svg class="w-10 h-10 mx-auto mb-3 text-muted-foreground/40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M17 8l4 4m0 0l-4 4m4-4H3"/>
        </svg>
        <p class="font-medium text-foreground mb-1">Email Forwarders</p>
        <p class="text-xs">Forwarder management is available in the per-site email settings.</p>
      </div>
    </div>
  {/if}

  <!-- ═══════════════════════════════════════════════════════════════════════ -->
  <!-- TAB: Client Setup                                                      -->
  <!-- ═══════════════════════════════════════════════════════════════════════ -->
  {#if activeTab === 'client-setup'}
    <div class="transition-all duration-200 space-y-4">
      <div class="bg-card border border-border rounded-xl overflow-hidden">
        <button
          on:click={() => showServerInfo = !showServerInfo}
          class="w-full flex items-center justify-between px-4 py-3 text-sm hover:bg-muted/30 transition-colors"
        >
          <div class="flex items-center gap-2 text-foreground font-medium">
            <svg class="w-4 h-4 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2"/>
            </svg>
            Mail Server Connection Info
          </div>
          <svg class="w-4 h-4 text-muted-foreground transition-transform {showServerInfo ? 'rotate-180' : ''}"
               fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
          </svg>
        </button>

        {#if showServerInfo}
          <div class="px-4 pb-4 border-t border-border">
            <p class="text-xs text-muted-foreground mt-3 mb-3">
              Replace <code class="bg-muted px-1 py-0.5 rounded text-foreground">{'{domain}'}</code>
              with the actual domain for the mailbox.
            </p>
            <div class="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <div class="bg-muted/40 rounded-lg px-3 py-2.5 space-y-1">
                <p class="text-xs font-medium text-foreground">IMAP (Incoming)</p>
                <p class="text-xs text-muted-foreground font-mono">mail.{'{domain}'}:993 — SSL/TLS</p>
              </div>
              <div class="bg-muted/40 rounded-lg px-3 py-2.5 space-y-1">
                <p class="text-xs font-medium text-foreground">SMTP (Outgoing)</p>
                <p class="text-xs text-muted-foreground font-mono">mail.{'{domain}'}:587 — STARTTLS</p>
              </div>
              <div class="bg-muted/40 rounded-lg px-3 py-2.5 space-y-1">
                <p class="text-xs font-medium text-foreground">POP3 (Incoming)</p>
                <p class="text-xs text-muted-foreground font-mono">mail.{'{domain}'}:995 — SSL/TLS</p>
              </div>
              <div class="bg-muted/40 rounded-lg px-3 py-2.5 space-y-1">
                <p class="text-xs font-medium text-foreground">Webmail</p>
                <p class="text-xs text-muted-foreground font-mono">https://mail.{'{domain}'}</p>
              </div>
            </div>
          </div>
        {/if}
      </div>
    </div>
  {/if}

  <!-- ═══════════════════════════════════════════════════════════════════════ -->
  <!-- TAB: SMTP Limits                                                       -->
  <!-- ═══════════════════════════════════════════════════════════════════════ -->
  {#if activeTab === 'smtp-limits'}
    <div class="transition-all duration-200 space-y-4">

      <!-- Header description -->
      <div class="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2">
        <div>
          <h2 class="text-base font-semibold text-foreground">SMTP Sending Limits</h2>
          <p class="text-sm text-muted-foreground mt-0.5">
            Control outbound email rate per domain to prevent spam and abuse.
          </p>
        </div>
        <button
          on:click={() => void loadSmtpLimits()}
          disabled={smtpLoading}
          class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-colors disabled:opacity-50"
        >
          <svg class="w-3.5 h-3.5 {smtpLoading ? 'animate-spin' : ''}" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
          </svg>
          Refresh
        </button>
      </div>

      <!-- Load error -->
      {#if smtpLoadError}
        <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                    bg-red-500/10 border border-red-500/20 text-sm text-red-400">
          <svg class="w-4 h-4 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
          </svg>
          {smtpLoadError}
        </div>
      {/if}

      <!-- Loading skeleton -->
      {#if smtpLoading}
        <div class="space-y-2">
          {#each [1,2,3] as _}
            <div class="h-16 bg-muted rounded-xl animate-pulse"></div>
          {/each}
        </div>

      <!-- Empty state -->
      {:else if smtpLimits.length === 0}
        <div class="bg-card border border-border rounded-xl p-10 text-center">
          <div class="w-12 h-12 rounded-full bg-muted/50 flex items-center justify-center mx-auto mb-3">
            <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z"/>
            </svg>
          </div>
          <p class="text-sm font-medium text-foreground mb-1">No SMTP limits configured</p>
          <p class="text-xs text-muted-foreground">
            Limits are created automatically when a domain is added, or you can add them manually per domain.
          </p>
        </div>

      <!-- Domain limits table -->
      {:else}
        <div class="rounded-xl border border-border overflow-x-auto">
          <table class="w-full text-sm">
            <thead class="bg-muted/50">
              <tr>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider">Domain</th>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider hidden sm:table-cell">Hourly</th>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider hidden md:table-cell">Daily</th>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider hidden lg:table-cell">Action</th>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider hidden lg:table-cell">Relay</th>
                <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase tracking-wider">Enabled</th>
                <th class="px-4 py-3 text-right text-xs text-muted-foreground uppercase tracking-wider">Actions</th>
              </tr>
            </thead>
            <tbody>
              {#each smtpLimits as limit}
                {@const hPct = limitPct(limit.sent_this_hour, limit.hourly_limit)}
                {@const dPct = limitPct(limit.sent_today, limit.daily_limit)}
                <tr class="border-b border-border last:border-0 hover:bg-muted/30 transition-colors duration-150">

                  <!-- Domain name -->
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-2">
                      <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
                        <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"/>
                        </svg>
                      </div>
                      <span class="font-mono text-sm text-foreground">{limit.domain}</span>
                    </div>
                  </td>

                  <!-- Hourly usage -->
                  <td class="px-4 py-3 hidden sm:table-cell">
                    <div class="space-y-1 min-w-[120px]">
                      <div class="flex items-center justify-between gap-2">
                        <div class="h-1.5 flex-1 bg-muted rounded-full overflow-hidden">
                          <div class="h-full rounded-full transition-all {limitColor(hPct)}"
                               style="width: {hPct}%"></div>
                        </div>
                      </div>
                      <p class="text-xs text-muted-foreground">
                        {limit.sent_this_hour} / {limit.hourly_limit === 0 ? '∞' : limit.hourly_limit}
                      </p>
                    </div>
                  </td>

                  <!-- Daily usage -->
                  <td class="px-4 py-3 hidden md:table-cell">
                    <div class="space-y-1 min-w-[120px]">
                      <div class="flex items-center justify-between gap-2">
                        <div class="h-1.5 flex-1 bg-muted rounded-full overflow-hidden">
                          <div class="h-full rounded-full transition-all {limitColor(dPct)}"
                               style="width: {dPct}%"></div>
                        </div>
                      </div>
                      <p class="text-xs text-muted-foreground">
                        {limit.sent_today} / {limit.daily_limit === 0 ? '∞' : limit.daily_limit}
                      </p>
                    </div>
                  </td>

                  <!-- Rate action badge -->
                  <td class="px-4 py-3 hidden lg:table-cell">
                    <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium border
                                 {rateActionBadge(limit.rate_action)}">
                      {limit.rate_action}
                    </span>
                  </td>

                  <!-- Relay toggle -->
                  <td class="px-4 py-3 hidden lg:table-cell">
                    <button
                      on:click={() => handleToggleRelay(limit)}
                      class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors
                             {limit.relay_enabled ? 'bg-primary' : 'bg-muted'}"
                      title="{limit.relay_enabled ? 'Relay enabled' : 'Relay disabled'}"
                    >
                      <span class="inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform
                                   {limit.relay_enabled ? 'translate-x-4.5' : 'translate-x-0.5'}"></span>
                    </button>
                  </td>

                  <!-- Enabled toggle -->
                  <td class="px-4 py-3">
                    <button
                      on:click={() => handleToggleEnabled(limit)}
                      class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors
                             {limit.enabled ? 'bg-primary' : 'bg-muted'}"
                      title="{limit.enabled ? 'Enabled' : 'Disabled'}"
                    >
                      <span class="inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform
                                   {limit.enabled ? 'translate-x-4.5' : 'translate-x-0.5'}"></span>
                    </button>
                  </td>

                  <!-- Actions -->
                  <td class="px-4 py-3">
                    <div class="flex items-center gap-1 justify-end">
                      <!-- Edit -->
                      <button
                        on:click={() => openEditModal(limit)}
                        title="Edit limits"
                        class="h-8 w-8 rounded-lg border border-border flex items-center justify-center
                               text-muted-foreground hover:bg-muted hover:text-foreground transition-colors"
                      >
                        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z"/>
                        </svg>
                      </button>

                      <!-- Reset counters -->
                      <button
                        on:click={() => handleResetCounters(limit.domain)}
                        disabled={resetLoading[limit.domain]}
                        title="Reset send counters"
                        class="h-8 px-2 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20
                               flex items-center gap-1 text-xs font-medium
                               hover:bg-red-500/20 transition-colors disabled:opacity-50"
                      >
                        <svg class="w-3 h-3 {resetLoading[limit.domain] ? 'animate-spin' : ''}"
                             fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
                        </svg>
                        <span class="hidden sm:inline">Reset</span>
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}

      <!-- ── Global Defaults card (collapsible) ─────────────────────────── -->
      <div class="bg-card border border-border rounded-xl overflow-hidden">
        <button
          on:click={() => showGlobalDefaults = !showGlobalDefaults}
          class="w-full flex items-center justify-between px-4 py-3 text-sm hover:bg-muted/30 transition-colors"
        >
          <div class="flex items-center gap-2 text-foreground font-medium">
            <svg class="w-4 h-4 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M12 6V4m0 2a2 2 0 100 4m0-4a2 2 0 110 4m-6 8a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4m6 6v10m6-2a2 2 0 100-4m0 4a2 2 0 110-4m0 4v2m0-6V4"/>
            </svg>
            Global Defaults
          </div>
          <svg class="w-4 h-4 text-muted-foreground transition-transform {showGlobalDefaults ? 'rotate-180' : ''}"
               fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7"/>
          </svg>
        </button>

        {#if showGlobalDefaults}
          <div class="px-4 pb-4 border-t border-border pt-4 space-y-4">
            <p class="text-xs text-muted-foreground">
              Default limits applied to newly added domains. Existing domains are not affected.
            </p>
            <div class="grid grid-cols-1 sm:grid-cols-3 gap-3">
              <div>
                <label class="block text-xs font-medium text-foreground mb-1.5">Hourly limit</label>
                <input
                  type="number"
                  bind:value={globalDefaults.hourly_limit}
                  min="10" max="50000"
                  class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                         focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                />
              </div>
              <div>
                <label class="block text-xs font-medium text-foreground mb-1.5">Daily limit</label>
                <input
                  type="number"
                  bind:value={globalDefaults.daily_limit}
                  min="10" max="1000000"
                  class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                         focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                />
              </div>
              <div>
                <label class="block text-xs font-medium text-foreground mb-1.5">When exceeded</label>
                <select
                  bind:value={globalDefaults.rate_action}
                  class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                         focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
                >
                  <option value="queue">Queue (hold and retry)</option>
                  <option value="reject">Reject (bounce with error)</option>
                  <option value="bounce">Bounce (send NDR)</option>
                </select>
              </div>
            </div>
            <button
              on:click={() => { globalDefaultsLoading = true; setTimeout(() => { globalDefaultsLoading = false; showToast('Global defaults saved', 'success'); }, 400); }}
              disabled={globalDefaultsLoading}
              class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                     hover:bg-primary/90 transition-colors disabled:opacity-50"
            >
              {globalDefaultsLoading ? 'Saving...' : 'Save Defaults'}
            </button>
          </div>
        {/if}
      </div>

    </div>
  {/if}

</div>

<!-- ── Create Email Modal ──────────────────────────────────────────────────────── -->
{#if showCreateModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="presentation"
    aria-hidden="true"
    on:click|self={() => showCreateModal = false}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl"
         role="dialog" aria-modal="true" aria-label="Create Email Account">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">Create Email Account</h2>
        <button
          on:click={() => showCreateModal = false}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <form on:submit={handleCreate} class="space-y-4">
        <div>
          <label for="create-domain" class="block text-sm font-medium text-foreground mb-1.5">Domain</label>
          <select id="create-domain" bind:value={createDomainId}
                  class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm
                         text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary">
            {#each domains as d}
              <option value={d.id}>@{d.domain}</option>
            {/each}
          </select>
        </div>

        <div>
          <label for="create-username" class="block text-sm font-medium text-foreground mb-1.5">Username</label>
          <div class="flex items-center border border-border rounded-lg overflow-hidden
                      focus-within:ring-2 focus-within:ring-primary/50 focus-within:border-primary">
            <input
              id="create-username"
              type="text"
              bind:value={createUsername}
              placeholder="username"
              required
              autocomplete="off"
              class="flex-1 h-9 px-3 bg-background text-sm text-foreground
                     focus:outline-none placeholder:text-muted-foreground"
            />
            <span class="px-3 text-sm text-muted-foreground bg-muted border-l border-border h-9 flex items-center shrink-0">
              @{selectedDomainName}
            </span>
          </div>
        </div>

        <div>
          <label for="create-password" class="block text-sm font-medium text-foreground mb-1.5">Password</label>
          <div class="flex gap-2">
            <div class="flex-1 relative">
              <input
                id="create-password"
                type={showPassword ? 'text' : 'password'}
                bind:value={createPassword}
                required
                autocomplete="new-password"
                class="w-full h-9 rounded-lg border border-border bg-background px-3 pr-9 text-sm
                       text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              />
              <button
                type="button"
                on:click={() => showPassword = !showPassword}
                class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                tabindex="-1"
              >
                {#if showPassword}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21"/>
                  </svg>
                {:else}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"/>
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"/>
                  </svg>
                {/if}
              </button>
            </div>
            <button
              type="button"
              on:click={() => createPassword = generatePassword()}
              title="Generate strong password"
              class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground
                     hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-colors"
            >
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"/>
              </svg>
              Generate
            </button>
          </div>
          {#if createPassword}
            <div class="mt-2 space-y-1">
              <div class="flex gap-1">
                {#each [1,2,3,4,5,6] as i}
                  <div class="flex-1 h-1 rounded-full transition-all
                              {i <= pwStrength.score ? pwStrength.color : 'bg-muted'}"></div>
                {/each}
              </div>
              <p class="text-xs text-muted-foreground">Strength: <span class="font-medium text-foreground">{pwStrength.label}</span></p>
            </div>
          {/if}
          <p class="text-xs text-muted-foreground mt-1">Min 10 chars — must include uppercase, lowercase, digit, and special character.</p>
        </div>

        <div>
          <label for="create-quota" class="block text-sm font-medium text-foreground mb-1.5">
            Storage Quota: <span class="text-primary font-semibold">{formatMb(createQuotaMb)}</span>
          </label>
          <input
            id="create-quota"
            type="range"
            bind:value={createQuotaMb}
            min="100" max="10240" step="100"
            class="w-full accent-primary"
          />
          <div class="flex justify-between text-xs text-muted-foreground mt-0.5">
            <span>100 MB</span><span>10 GB</span>
          </div>
        </div>

        {#if createError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 rounded-lg px-3 py-2.5 border border-red-500/20">
            {createError}
          </div>
        {/if}

        <div class="flex gap-2 pt-1">
          <button
            type="submit"
            disabled={createLoading}
            class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                   hover:bg-primary/90 transition-colors disabled:opacity-50"
          >
            {createLoading ? 'Creating...' : 'Create Account'}
          </button>
          <button
            type="button"
            on:click={() => showCreateModal = false}
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                   hover:bg-muted hover:text-foreground transition-colors"
          >
            Cancel
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<!-- ── Edit SMTP Limits Modal ──────────────────────────────────────────────────── -->
{#if showEditModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="presentation"
    aria-hidden="true"
    on:click|self={() => { if (!editLoading) showEditModal = false; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl"
         role="dialog" aria-modal="true" aria-label="Edit SMTP Limits">
      <div class="flex items-center justify-between mb-5">
        <div>
          <h2 class="text-base font-semibold text-foreground">Edit SMTP Limits</h2>
          <p class="text-xs text-muted-foreground font-mono mt-0.5">{editDomain}</p>
        </div>
        <button
          on:click={() => showEditModal = false}
          disabled={editLoading}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <div class="space-y-5">
        <!-- Hourly limit -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">Hourly limit</label>
          <input
            type="number"
            bind:value={editForm.hourly_limit}
            min="10" max="50000"
            class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                   focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
          />
          <div class="flex flex-wrap gap-1.5 mt-2">
            {#each [100, 500, 1000, 5000] as preset}
              <button
                type="button"
                on:click={() => setPreset('hourly_limit', preset)}
                class="h-7 px-2.5 rounded-md text-xs border border-border text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-colors
                       {editForm.hourly_limit === preset ? 'bg-primary/10 border-primary/30 text-primary' : ''}"
              >{preset}</button>
            {/each}
            <button
              type="button"
              on:click={() => setPreset('hourly_limit', 0)}
              class="h-7 px-2.5 rounded-md text-xs border border-border text-muted-foreground
                     hover:bg-muted hover:text-foreground transition-colors
                     {editForm.hourly_limit === 0 ? 'bg-primary/10 border-primary/30 text-primary' : ''}"
            >Unlimited</button>
          </div>
        </div>

        <!-- Daily limit -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5">Daily limit</label>
          <input
            type="number"
            bind:value={editForm.daily_limit}
            min="10" max="1000000"
            class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground
                   focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
          />
          <div class="flex flex-wrap gap-1.5 mt-2">
            {#each [500, 2000, 5000, 20000] as preset}
              <button
                type="button"
                on:click={() => setPreset('daily_limit', preset)}
                class="h-7 px-2.5 rounded-md text-xs border border-border text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-colors
                       {editForm.daily_limit === preset ? 'bg-primary/10 border-primary/30 text-primary' : ''}"
              >{preset}</button>
            {/each}
            <button
              type="button"
              on:click={() => setPreset('daily_limit', 0)}
              class="h-7 px-2.5 rounded-md text-xs border border-border text-muted-foreground
                     hover:bg-muted hover:text-foreground transition-colors
                     {editForm.daily_limit === 0 ? 'bg-primary/10 border-primary/30 text-primary' : ''}"
            >Unlimited</button>
          </div>
        </div>

        <!-- When limit exceeded — radio cards -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-2">When limit exceeded</label>
          <div class="grid grid-cols-3 gap-2">
            {#each [
              { value: 'queue',  label: 'Queue',  desc: 'Hold and retry' },
              { value: 'reject', label: 'Reject', desc: 'Bounce with error' },
              { value: 'bounce', label: 'Bounce', desc: 'Send NDR' },
            ] as opt}
              <button
                type="button"
                on:click={() => editForm = { ...editForm, rate_action: opt.value as 'queue' | 'reject' | 'bounce' }}
                class="rounded-lg border px-3 py-2.5 text-left transition-colors
                       {editForm.rate_action === opt.value
                         ? 'border-primary bg-primary/5'
                         : 'border-border hover:bg-muted'}"
              >
                <p class="text-xs font-semibold text-foreground">{opt.label}</p>
                <p class="text-xs text-muted-foreground mt-0.5">{opt.desc}</p>
              </button>
            {/each}
          </div>
        </div>

        <!-- Toggle row -->
        <div class="space-y-3">
          {#each [
            { key: 'relay_enabled' as keyof SmtpLimitsRequest, label: 'Relay enabled', desc: 'Allow sending from this domain through our SMTP server' },
            { key: 'require_spf'   as keyof SmtpLimitsRequest, label: 'Require SPF',    desc: 'Reject emails if sender IP not in SPF record' },
            { key: 'require_dkim'  as keyof SmtpLimitsRequest, label: 'Require DKIM',   desc: 'Reject emails not signed with DKIM' },
            { key: 'enabled'       as keyof SmtpLimitsRequest, label: 'Enabled',         desc: 'Disable to block all outbound from this domain' },
          ] as row}
            <label class="flex items-center justify-between gap-3 cursor-pointer">
              <div>
                <p class="text-sm font-medium text-foreground">{row.label}</p>
                <p class="text-xs text-muted-foreground">{row.desc}</p>
              </div>
              <button
                type="button"
                on:click={() => editForm = { ...editForm, [row.key]: !editForm[row.key] }}
                class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors
                       {editForm[row.key] ? 'bg-primary' : 'bg-muted'}"
              >
                <span class="inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform
                             {editForm[row.key] ? 'translate-x-4.5' : 'translate-x-0.5'}"></span>
              </button>
            </label>
          {/each}
        </div>

        {#if editError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 rounded-lg px-3 py-2.5 border border-red-500/20">
            {editError}
          </div>
        {/if}

        <div class="flex gap-2 pt-1">
          <button
            type="button"
            on:click={handleSaveLimits}
            disabled={editLoading}
            class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                   hover:bg-primary/90 transition-colors disabled:opacity-50"
          >
            {editLoading ? 'Saving...' : 'Save Limits'}
          </button>
          <button
            type="button"
            on:click={() => showEditModal = false}
            disabled={editLoading}
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                   hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

<!-- ── Delete Confirmation Modal ──────────────────────────────────────────────── -->
{#if deleteTarget}
  {@const addr = deleteTarget.address ?? `${deleteTarget.local_part ?? ''}@${accountDomain(deleteTarget)}`}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="presentation"
    aria-hidden="true"
    on:click|self={() => { if (!deleteLoading) deleteTarget = null; }}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl"
         role="dialog" aria-modal="true">
      <div class="flex items-start gap-3 mb-4">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"/>
          </svg>
        </div>
        <div>
          <h2 class="text-base font-semibold text-foreground">Delete Email Account</h2>
          <p class="text-sm text-muted-foreground mt-0.5">
            This will permanently delete <span class="font-mono text-foreground">{addr}</span> and all its emails.
          </p>
        </div>
      </div>

      <label class="flex items-start gap-2.5 cursor-pointer mb-5">
        <input type="checkbox" bind:checked={deleteConfirmCheck} class="mt-0.5 accent-red-500"/>
        <span class="text-sm text-foreground">I understand this will delete all emails in this mailbox and cannot be undone.</span>
      </label>

      <div class="flex gap-2">
        <button
          on:click={handleDelete}
          disabled={!deleteConfirmCheck || deleteLoading}
          class="flex-1 h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20
                 text-sm font-medium hover:bg-red-500/20 inline-flex items-center justify-center gap-2
                 transition-colors disabled:opacity-40"
        >
          {deleteLoading ? 'Deleting...' : 'Delete Account'}
        </button>
        <button
          on:click={() => deleteTarget = null}
          disabled={deleteLoading}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50"
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ───────────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
              flex items-center gap-2"
       role="status" aria-live="polite">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 text-green-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/>
      </svg>
    {:else}
      <svg class="w-4 h-4 text-red-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}
