<script lang="ts">
  import { onMount, createEventDispatcher } from 'svelte';
  import { api } from '$api/client';
  import type { EmailAccount, EmailForwarder, EmailDomain, Site } from '$api/client';

  export let siteId: string;
  export let site: Site;

  const dispatch = createEventDispatcher<{ tabChange: string }>();

  // ── Sub-tab state ────────────────────────────────────────────────────────────
  type SubTab = 'accounts' | 'forwarders' | 'setup' | 'health';
  let activeTab: SubTab = 'accounts';

  // ── Data ─────────────────────────────────────────────────────────────────────
  let accounts: EmailAccount[] = [];
  let forwarders: EmailForwarder[] = [];
  let domains: EmailDomain[] = [];
  let loading = true;
  let loadingAccounts = true;
  let loadingForwarders = true;
  let loadingDomains = true;

  // ── Toast ─────────────────────────────────────────────────────────────────────
  interface Toast { id: number; msg: string; type: 'ok' | 'err' }
  let toasts: Toast[] = [];
  let toastSeq = 0;
  function toast(msg: string, type: Toast['type'] = 'ok') {
    const id = ++toastSeq;
    toasts = [...toasts, { id, msg, type }];
    setTimeout(() => { toasts = toasts.filter(t => t.id !== id); }, 4000);
  }

  // ── Helpers ───────────────────────────────────────────────────────────────────
  function emailAddress(account: EmailAccount): string {
    return account.address ?? (account.local_part ?? '') + '@' + site.domain;
  }

  function quotaColor(used: number, quota: number): string {
    if (quota === 0) return 'bg-green-500';
    const pct = used / quota;
    if (pct >= 0.9) return 'bg-red-500';
    if (pct >= 0.7) return 'bg-yellow-500';
    return 'bg-green-500';
  }

  function quotaPct(used: number, quota: number): number {
    if (quota === 0) return 0;
    return Math.min(100, Math.round((used / quota) * 100));
  }

  // ── Validation helpers ────────────────────────────────────────────────────────
  function isValidEmail(addr: string): boolean {
    return /^[^\s@]+@[^\s@]+\.[^\s@]{2,}$/.test(addr.trim());
  }

  function isValidLocalPart(lp: string): boolean {
    return /^[a-z0-9]([a-z0-9._+-]*[a-z0-9])?$/i.test(lp.trim()) && lp.trim().length >= 1;
  }

  // ── Password utilities ────────────────────────────────────────────────────────
  function generatePassword(len = 14): string {
    const chars = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%';
    const arr = new Uint8Array(len);
    crypto.getRandomValues(arr);
    return Array.from(arr, b => chars[b % chars.length]).join('');
  }

  function pwStrength(pw: string): number {
    let s = 0;
    if (pw.length >= 10) s++;
    if (/[A-Z]/.test(pw)) s++;
    if (/[0-9]/.test(pw)) s++;
    if (/[^A-Za-z0-9]/.test(pw)) s++;
    return s;
  }

  const strengthLabel = ['', 'Weak', 'Fair', 'Good', 'Strong'];
  const strengthColor = ['', 'bg-red-500', 'bg-yellow-500', 'bg-blue-500', 'bg-green-500'];

  // ── Modal: Create Account ─────────────────────────────────────────────────────
  let showCreateAccount = false;
  let newLocalPart = '';
  let newPassword = generatePassword();
  let newShowPw = false;
  let newQuota = 500;
  let creatingAccount = false;

  function openCreateAccount() {
    newLocalPart = '';
    newPassword = generatePassword();
    newShowPw = false;
    newQuota = 500;
    showCreateAccount = true;
  }

  async function submitCreateAccount() {
    if (!newLocalPart.trim() || !newPassword) return;
    if (!isValidLocalPart(newLocalPart)) {
      toast('Invalid local part — use letters, numbers, dots, underscores and hyphens only', 'err');
      return;
    }
    const domainId = domains[0]?.id;
    if (!domainId) { toast('No email domain configured', 'err'); return; }
    creatingAccount = true;
    try {
      const acct = await api.email.accounts.create({
        domain_id: domainId,
        local_part: newLocalPart.trim(),
        password: newPassword,
        quota_mb: newQuota,
      });
      accounts = [...accounts, acct];
      showCreateAccount = false;
      toast('Account created');
    } catch (e: unknown) {
      toast(e instanceof Error ? e.message : 'Failed to create account', 'err');
    } finally {
      creatingAccount = false;
    }
  }

  // ── Modal: Change Password ────────────────────────────────────────────────────
  let showChangePw = false;
  let changePwAccount: EmailAccount | null = null;
  let changePwValue = '';
  let changePwShow = false;
  let changingPw = false;

  function openChangePw(account: EmailAccount) {
    changePwAccount = account;
    changePwValue = generatePassword();
    changePwShow = false;
    showChangePw = true;
  }

  async function submitChangePw() {
    if (!changePwAccount || !changePwValue) return;
    changingPw = true;
    try {
      await api.email.accounts.changePassword(changePwAccount.id, changePwValue);
      showChangePw = false;
      toast('Password changed');
    } catch (e: unknown) {
      toast(e instanceof Error ? e.message : 'Failed to change password', 'err');
    } finally {
      changingPw = false;
    }
  }

  // ── Delete Account ────────────────────────────────────────────────────────────
  let deletingAccountId: string | null = null;
  let deleteAccountTarget: EmailAccount | null = null;
  let deleteAccountConfirm = '';

  function deleteAccount(account: EmailAccount) {
    deleteAccountTarget = account;
    deleteAccountConfirm = '';
  }
  function closeDeleteAccount() {
    deleteAccountTarget = null;
    deleteAccountConfirm = '';
  }
  async function confirmDeleteAccount() {
    if (!deleteAccountTarget) return;
    const fullAddr = emailAddress(deleteAccountTarget);
    if (deleteAccountConfirm !== fullAddr) return;
    deletingAccountId = deleteAccountTarget.id;
    try {
      await api.email.accounts.delete(deleteAccountTarget.id);
      accounts = accounts.filter(a => a.id !== deleteAccountTarget!.id);
      toast('Account deleted');
      closeDeleteAccount();
    } catch (e: unknown) {
      toast(e instanceof Error ? e.message : 'Failed to delete account', 'err');
    } finally {
      deletingAccountId = null;
    }
  }

  // ── Webmail SSO ───────────────────────────────────────────────────────────────
  async function openWebmail(account: EmailAccount) {
    try {
      const { sso_url } = await api.email.webmailSso(account.id);
      window.open(sso_url, '_blank', 'noopener');
    } catch (e: unknown) {
      toast(e instanceof Error ? e.message : 'Webmail SSO failed', 'err');
    }
  }

  // ── Modal: Create Forwarder ───────────────────────────────────────────────────
  let showCreateFwd = false;
  let fwdSource = '';
  let fwdDest = '';
  let creatingFwd = false;

  function openCreateFwd() {
    fwdSource = '';
    fwdDest = '';
    showCreateFwd = true;
  }

  async function submitCreateFwd() {
    if (!fwdSource.trim() || !fwdDest.trim()) return;
    if (!isValidEmail(fwdDest)) {
      toast('Destination must be a valid email address', 'err');
      return;
    }
    creatingFwd = true;
    try {
      const fwd = await api.email.forwarders.create({
        site_id: siteId,
        source: fwdSource.trim(),
        destination: fwdDest.trim(),
      });
      forwarders = [...forwarders, fwd];
      showCreateFwd = false;
      toast('Forwarder created');
    } catch (e: unknown) {
      toast(e instanceof Error ? e.message : 'Failed to create forwarder', 'err');
    } finally {
      creatingFwd = false;
    }
  }

  // ── Delete Forwarder ──────────────────────────────────────────────────────────
  let deletingFwdId: string | null = null;
  let confirmDeleteFwdId: string | null = null;

  async function deleteForwarder(fwd: EmailForwarder) {
    deletingFwdId = fwd.id;
    try {
      await api.email.forwarders.delete(fwd.id);
      forwarders = forwarders.filter(f => f.id !== fwd.id);
      confirmDeleteFwdId = null;
      toast('Forwarder deleted');
    } catch (e: unknown) {
      toast(e instanceof Error ? e.message : 'Failed to delete forwarder', 'err');
    } finally {
      deletingFwdId = null;
    }
  }

  // ── Copy helper ───────────────────────────────────────────────────────────────
  async function copyText(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toast('Copied to clipboard');
    } catch {
      toast('Copy failed', 'err');
    }
  }

  // ── Escape / click-outside ────────────────────────────────────────────────────
  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      showCreateAccount = false;
      showChangePw = false;
      showCreateFwd = false;
    }
  }

  function closeIfBackdrop(e: MouseEvent, closeFn: () => void) {
    if ((e.target as HTMLElement).dataset.backdrop) closeFn();
  }

  // ── onMount ───────────────────────────────────────────────────────────────────
  onMount(() => {
    Promise.all([
      api.email.accounts.list({ site_id: siteId })
        .then(r => { accounts = r; loadingAccounts = false; })
        .catch(() => { loadingAccounts = false; }),
      api.email.forwarders.list({ site_id: siteId })
        .then(r => { forwarders = r; loadingForwarders = false; })
        .catch(() => { loadingForwarders = false; }),
      api.email.domains.list({ site_id: siteId })
        .then(r => { domains = r; loadingDomains = false; })
        .catch(() => { loadingDomains = false; }),
    ]).finally(() => { loading = false; });
  });

  const subTabs: { id: SubTab; label: string }[] = [
    { id: 'accounts',   label: 'Accounts' },
    { id: 'forwarders', label: 'Forwarders' },
    { id: 'setup',      label: 'Client Setup' },
    { id: 'health',     label: 'Domain Health' },
  ];

  // ── Client setup cards ────────────────────────────────────────────────────────
  const clientCards = [
    {
      name: 'Thunderbird',
      icon: '📨',
      steps: [
        'Open File → New → Existing Mail Account',
        'Enter your name, email address, and password',
        'Click "Configure manually" and enter the server settings above',
      ],
    },
    {
      name: 'Outlook',
      icon: '📧',
      steps: [
        'Go to File → Add Account → Manual Setup',
        'Select "IMAP" and enter your email address',
        'Fill in incoming/outgoing server details from above',
      ],
    },
    {
      name: 'Apple Mail / iOS',
      icon: '🍎',
      steps: [
        'Go to Settings → Mail → Add Account → Other',
        'Tap "Add Mail Account" and enter your details',
        'Choose IMAP and enter server settings above',
      ],
    },
    {
      name: 'Android / Gmail',
      icon: '🤖',
      steps: [
        'Open Gmail app → tap menu → Add Account → Other',
        'Enter your email address and tap "Manual setup"',
        'Select IMAP and enter the server details above',
      ],
    },
  ];
</script>

<svelte:window on:keydown={handleKeydown} />

<!-- ── Toast container ──────────────────────────────────────────────────────── -->
<div class="fixed bottom-4 right-4 z-[100] flex flex-col gap-2 pointer-events-none">
  {#each toasts as t (t.id)}
    <div
      class="pointer-events-auto px-4 py-3 rounded-lg text-sm font-medium shadow-lg transition-all
        {t.type === 'ok' ? 'bg-green-500/90 text-white' : 'bg-red-500/90 text-white'}"
    >
      {t.msg}
    </div>
  {/each}
</div>

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
</style>

<!-- ── Main layout ───────────────────────────────────────────────────────────── -->
<div class="tab-content space-y-4">
  <!-- Sub-tab pills -->
  <div class="flex items-center gap-1 p-1 bg-muted/30 rounded-xl w-fit">
    {#each subTabs as tab}
      <button
        class="px-3 py-1.5 rounded-lg text-sm font-medium transition-colors
          {activeTab === tab.id
            ? 'bg-primary text-primary-foreground'
            : 'text-muted-foreground hover:text-foreground hover:bg-muted'}"
        on:click={() => (activeTab = tab.id)}
      >
        {tab.label}
      </button>
    {/each}
  </div>

  <!-- ── ACCOUNTS TAB ────────────────────────────────────────────────────────── -->
  {#if activeTab === 'accounts'}
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <!-- Header -->
      <div class="flex items-center justify-between px-5 py-4 border-b border-border">
        <div>
          <h3 class="text-sm font-semibold text-foreground">Email Accounts</h3>
          <p class="text-xs text-muted-foreground mt-0.5">{accounts.length} account{accounts.length !== 1 ? 's' : ''}</p>
        </div>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
          on:click={openCreateAccount}
        >
          + New Account
        </button>
      </div>

      <!-- Skeleton -->
      {#if loadingAccounts}
        <div class="divide-y divide-border">
          {#each [1, 2, 3] as _}
            <div class="px-5 py-4 flex items-center gap-4 animate-pulse">
              <div class="h-4 w-44 bg-muted rounded"></div>
              <div class="h-2 flex-1 bg-muted rounded"></div>
              <div class="h-6 w-16 bg-muted rounded-full"></div>
              <div class="h-8 w-24 bg-muted rounded"></div>
            </div>
          {/each}
        </div>

      {:else if accounts.length === 0}
        <!-- Empty state -->
        <div class="flex flex-col items-center justify-center py-14 gap-3 text-center">
          <div class="w-14 h-14 rounded-full bg-muted/50 flex items-center justify-center text-2xl">✉️</div>
          <p class="text-sm font-medium text-foreground">No email accounts yet</p>
          <p class="text-xs text-muted-foreground max-w-xs">
            Create an account to send and receive email at @{site.domain}
          </p>
          <button
            class="mt-2 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
            on:click={openCreateAccount}
          >
            Create first account
          </button>
        </div>

      {:else}
        <!-- Table -->
        <div class="divide-y divide-border">
          <!-- Head -->
          <div class="grid grid-cols-[1fr_180px_80px_auto] gap-4 px-5 py-2 text-xs text-muted-foreground uppercase tracking-wide">
            <span>Address</span>
            <span>Quota</span>
            <span>Status</span>
            <span>Actions</span>
          </div>
          {#each accounts as account (account.id)}
            <div class="grid grid-cols-[1fr_180px_80px_auto] gap-4 items-center px-5 py-3 hover:bg-muted/10 transition-colors">
              <!-- Address -->
              <span class="font-mono text-sm text-foreground truncate">{emailAddress(account)}</span>

              <!-- Quota bar -->
              <div class="flex flex-col gap-1">
                <span class="text-xs text-muted-foreground">
                  {account.used_mb} / {account.quota_mb === 0 ? '∞' : account.quota_mb + ' MB'}
                </span>
                {#if account.quota_mb > 0}
                  <div class="h-1.5 w-full bg-muted rounded-full overflow-hidden">
                    <div
                      class="h-full rounded-full transition-all {quotaColor(account.used_mb, account.quota_mb)}"
                      style="width: {quotaPct(account.used_mb, account.quota_mb)}%"
                    ></div>
                  </div>
                {:else}
                  <div class="h-1.5 w-full bg-muted rounded-full"></div>
                {/if}
              </div>

              <!-- Status badge -->
              <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium
                {account.status === 'active'
                  ? 'bg-green-500/10 text-green-400'
                  : 'bg-red-500/10 text-red-400'}">
                {account.status}
              </span>

              <!-- Actions -->
              <div class="flex items-center gap-1">
                <button
                  class="h-8 w-8 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted transition-colors flex items-center justify-center"
                  title="Open Webmail"
                  on:click={() => openWebmail(account)}
                >
                  🌐
                </button>
                <button
                  class="h-8 w-8 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted transition-colors flex items-center justify-center"
                  title="Change Password"
                  on:click={() => openChangePw(account)}
                >
                  🔑
                </button>
                <button
                  class="h-8 w-8 rounded-lg text-sm text-red-400 hover:bg-red-500/10 transition-colors flex items-center justify-center"
                  title="Delete Account"
                  disabled={deletingAccountId === account.id}
                  on:click={() => deleteAccount(account)}
                >
                  {deletingAccountId === account.id ? '…' : '🗑️'}
                </button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <!-- ── FORWARDERS TAB ──────────────────────────────────────────────────────── -->
  {#if activeTab === 'forwarders'}
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="flex items-center justify-between px-5 py-4 border-b border-border">
        <div>
          <h3 class="text-sm font-semibold text-foreground">Forwarders</h3>
          <p class="text-xs text-muted-foreground mt-0.5">{forwarders.length} forwarder{forwarders.length !== 1 ? 's' : ''}</p>
        </div>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
          on:click={openCreateFwd}
        >
          + New Forwarder
        </button>
      </div>

      {#if loadingForwarders}
        <div class="divide-y divide-border">
          {#each [1, 2] as _}
            <div class="px-5 py-4 flex items-center gap-4 animate-pulse">
              <div class="h-4 w-44 bg-muted rounded"></div>
              <div class="h-4 w-6 bg-muted rounded"></div>
              <div class="h-4 w-44 bg-muted rounded"></div>
              <div class="ml-auto h-8 w-8 bg-muted rounded"></div>
            </div>
          {/each}
        </div>

      {:else if forwarders.length === 0}
        <div class="flex flex-col items-center justify-center py-14 gap-3 text-center">
          <div class="w-14 h-14 rounded-full bg-muted/50 flex items-center justify-center text-2xl">↪️</div>
          <p class="text-sm font-medium text-foreground">No forwarders configured</p>
          <p class="text-xs text-muted-foreground max-w-xs">
            Forward emails from one address to another, or set a catch-all.
          </p>
          <button
            class="mt-2 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
            on:click={openCreateFwd}
          >
            Create forwarder
          </button>
        </div>

      {:else}
        <div class="divide-y divide-border">
          <div class="grid grid-cols-[1fr_24px_1fr_auto] gap-4 px-5 py-2 text-xs text-muted-foreground uppercase tracking-wide">
            <span>From</span>
            <span></span>
            <span>To</span>
            <span>Actions</span>
          </div>
          {#each forwarders as fwd (fwd.id)}
            <div class="grid grid-cols-[1fr_24px_1fr_auto] gap-4 items-center px-5 py-3 hover:bg-muted/10 transition-colors">
              <span class="font-mono text-sm text-foreground truncate">{fwd.source}</span>
              <span class="text-muted-foreground text-sm text-center">→</span>
              <span class="font-mono text-sm text-muted-foreground truncate">{fwd.destination}</span>
              {#if confirmDeleteFwdId === fwd.id}
                <div class="flex items-center gap-1">
                  <button class="text-xs px-2 py-0.5 rounded bg-destructive text-white hover:bg-destructive/90 disabled:opacity-50" disabled={deletingFwdId === fwd.id} on:click={() => deleteForwarder(fwd)}>{deletingFwdId === fwd.id ? '…' : 'Delete'}</button>
                  <button class="text-xs px-2 py-0.5 rounded bg-muted" on:click={() => confirmDeleteFwdId = null}>Cancel</button>
                </div>
              {:else}
                <button
                  class="h-8 w-8 rounded-lg text-sm text-red-400 hover:bg-red-500/10 transition-colors flex items-center justify-center"
                  title="Delete Forwarder"
                  on:click={() => confirmDeleteFwdId = fwd.id}
                >
                  🗑️
                </button>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <!-- ── CLIENT SETUP TAB ────────────────────────────────────────────────────── -->
  {#if activeTab === 'setup'}
    <!-- Server settings card -->
    <div class="bg-card border border-border rounded-xl p-5 space-y-4">
      <h3 class="text-sm font-semibold text-foreground">Mail Server Settings</h3>
      <div class="grid gap-3">
        {#each [
          { label: 'Incoming (IMAP)', value: `imap.${site.domain}`, note: 'Port 993 · SSL/TLS' },
          { label: 'Outgoing (SMTP)', value: `smtp.${site.domain}`, note: 'Port 587 · STARTTLS' },
          { label: 'Authentication', value: 'Normal password', note: '' },
          { label: 'Username', value: `user@${site.domain}`, note: 'Full email address' },
        ] as row}
          <div class="flex items-center justify-between gap-4 py-2 border-b border-border last:border-0">
            <div class="min-w-[160px]">
              <span class="text-xs text-muted-foreground">{row.label}</span>
              {#if row.note}
                <span class="text-xs text-muted-foreground/60 ml-1">· {row.note}</span>
              {/if}
            </div>
            <div class="flex items-center gap-2 flex-1 justify-end">
              <span class="font-mono text-sm text-foreground">{row.value}</span>
              <button
                class="h-7 px-2 rounded border border-border text-xs text-muted-foreground hover:bg-muted transition-colors"
                on:click={() => copyText(row.value)}
                title="Copy"
              >
                Copy
              </button>
            </div>
          </div>
        {/each}
      </div>
    </div>

    <!-- Client cards grid -->
    <div class="grid grid-cols-1 sm:grid-cols-2 gap-4">
      {#each clientCards as card}
        <div class="bg-card border border-border rounded-xl p-5 space-y-3">
          <div class="flex items-center gap-2">
            <span class="text-2xl">{card.icon}</span>
            <h4 class="text-sm font-semibold text-foreground">{card.name}</h4>
          </div>
          <ol class="space-y-1.5">
            {#each card.steps as step, i}
              <li class="flex gap-2 text-xs text-muted-foreground">
                <span class="shrink-0 w-4 h-4 rounded-full bg-muted flex items-center justify-center text-[10px] font-bold text-foreground">{i + 1}</span>
                <span>{step}</span>
              </li>
            {/each}
          </ol>
          <div class="relative group inline-block">
            <button
              class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground opacity-50 cursor-not-allowed transition-colors"
              disabled
            >
              Download Config
            </button>
            <div class="absolute bottom-full left-0 mb-1.5 px-2 py-1 bg-popover border border-border rounded text-xs text-muted-foreground whitespace-nowrap opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none z-10">
              Coming soon
            </div>
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <!-- ── DOMAIN HEALTH TAB ───────────────────────────────────────────────────── -->
  {#if activeTab === 'health'}
    {#if loadingDomains}
      <div class="space-y-3">
        {#each [1, 2, 3, 4] as _}
          <div class="bg-card border border-border rounded-xl px-5 py-4 animate-pulse flex items-center gap-4">
            <div class="h-5 w-5 rounded-full bg-muted"></div>
            <div class="h-4 w-32 bg-muted rounded"></div>
            <div class="ml-auto h-4 w-20 bg-muted rounded"></div>
          </div>
        {/each}
      </div>

    {:else if domains.length === 0}
      <div class="bg-card border border-border rounded-xl flex flex-col items-center justify-center py-14 gap-3 text-center">
        <div class="w-14 h-14 rounded-full bg-muted/50 flex items-center justify-center text-2xl">🔍</div>
        <p class="text-sm font-medium text-foreground">No email domain found</p>
        <p class="text-xs text-muted-foreground max-w-xs">Add an email domain first to view DNS health checks.</p>
      </div>

    {:else}
      {#each domains as domain}
        <div class="bg-card border border-border rounded-xl overflow-hidden mb-4">
          <div class="px-5 py-3 border-b border-border">
            <span class="text-xs text-muted-foreground font-medium uppercase tracking-wide">Domain:</span>
            <span class="ml-2 font-mono text-sm text-foreground">{domain.domain}</span>
          </div>

          <div class="divide-y divide-border">
            <!-- SPF -->
            <div class="flex items-center justify-between px-5 py-4">
              <div class="flex items-center gap-3">
                <span class="text-lg">{domain.spf_record !== null ? '✅' : '❌'}</span>
                <div>
                  <p class="text-sm font-medium text-foreground">SPF Record</p>
                  {#if domain.spf_record !== null}
                    <p class="text-xs text-muted-foreground font-mono truncate max-w-xs">{domain.spf_record}</p>
                  {:else}
                    <p class="text-xs text-muted-foreground">No SPF record detected</p>
                  {/if}
                </div>
              </div>
              <span class="px-2 py-0.5 rounded-full text-xs font-medium {domain.spf_record !== null ? 'bg-green-500/10 text-green-400' : 'bg-red-500/10 text-red-400'}">
                {domain.spf_record !== null ? 'Configured' : 'Missing'}
              </span>
            </div>

            <!-- DKIM -->
            <div class="flex items-center justify-between px-5 py-4">
              <div class="flex items-center gap-3">
                <span class="text-lg">{domain.dkim_selector ? '✅' : '❌'}</span>
                <div>
                  <p class="text-sm font-medium text-foreground">DKIM</p>
                  {#if domain.dkim_selector}
                    <p class="text-xs text-muted-foreground">Selector: <span class="font-mono">{domain.dkim_selector}</span></p>
                  {:else}
                    <p class="text-xs text-muted-foreground">DKIM selector not configured</p>
                  {/if}
                </div>
              </div>
              <span class="px-2 py-0.5 rounded-full text-xs font-medium {domain.dkim_selector ? 'bg-green-500/10 text-green-400' : 'bg-red-500/10 text-red-400'}">
                {domain.dkim_selector ? 'Configured' : 'Missing'}
              </span>
            </div>

            <!-- DMARC -->
            <div class="flex items-center justify-between px-5 py-4">
              <div class="flex items-center gap-3">
                <span class="text-lg">{domain.dmarc_record !== null ? '✅' : '❌'}</span>
                <div>
                  <p class="text-sm font-medium text-foreground">DMARC</p>
                  {#if domain.dmarc_record !== null}
                    <p class="text-xs text-muted-foreground font-mono truncate max-w-xs">{domain.dmarc_record}</p>
                  {:else}
                    <p class="text-xs text-muted-foreground">No DMARC record detected</p>
                  {/if}
                </div>
              </div>
              <span class="px-2 py-0.5 rounded-full text-xs font-medium {domain.dmarc_record !== null ? 'bg-green-500/10 text-green-400' : 'bg-red-500/10 text-red-400'}">
                {domain.dmarc_record !== null ? 'Configured' : 'Missing'}
              </span>
            </div>

            <!-- MX -->
            <div class="flex items-center justify-between px-5 py-4">
              <div class="flex items-center gap-3">
                <span class="text-lg">ℹ️</span>
                <div>
                  <p class="text-sm font-medium text-foreground">MX Records</p>
                  <p class="text-xs text-muted-foreground">Check DNS tab to verify MX records point to your server</p>
                </div>
              </div>
              <button
                class="h-8 px-3 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors"
                on:click={() => dispatch('tabChange', 'dns')}
              >
                Go to DNS →
              </button>
            </div>
          </div>
        </div>
      {/each}

      <!-- Setup Email Authentication CTA -->
      {#if domains.some(d => !d.spf_record || !d.dkim_selector || !d.dmarc_record)}
        <div class="bg-yellow-500/5 border border-yellow-500/20 rounded-xl px-5 py-4 flex items-center justify-between gap-4">
          <div>
            <p class="text-sm font-medium text-foreground">Setup Email Authentication</p>
            <p class="text-xs text-muted-foreground mt-0.5">Add SPF, DKIM and DMARC records to improve deliverability and prevent spoofing.</p>
          </div>
          <button
            class="shrink-0 h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
            on:click={() => dispatch('tabChange', 'dns')}
          >
            Configure DNS
          </button>
        </div>
      {/if}
    {/if}
  {/if}
</div>

<!-- ── MODAL: Create Account ───────────────────────────────────────────────── -->
{#if showCreateAccount}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    data-backdrop="1"
    on:click={(e) => closeIfBackdrop(e, () => (showCreateAccount = false))}
  >
    <div class="bg-card border border-border rounded-xl w-full max-w-md mx-4 p-6 space-y-5 shadow-2xl">
      <div class="flex items-center justify-between">
        <h3 class="text-base font-semibold text-foreground">New Email Account</h3>
        <button
          class="h-7 w-7 rounded-md text-muted-foreground hover:bg-muted transition-colors flex items-center justify-center text-lg"
          on:click={() => (showCreateAccount = false)}
        >×</button>
      </div>

      <!-- Local part + domain -->
      <div class="space-y-1">
        <label class="text-xs font-medium text-muted-foreground">Email Address</label>
        <div class="flex items-center gap-0">
          <input
            class="flex-1 h-10 px-3 rounded-l-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
            type="text"
            placeholder="username"
            bind:value={newLocalPart}
          />
          <span class="h-10 px-3 flex items-center bg-muted border border-l-0 border-border rounded-r-lg text-sm text-muted-foreground whitespace-nowrap">
            @{site.domain}
          </span>
        </div>
      </div>

      <!-- Password -->
      <div class="space-y-1">
        <label class="text-xs font-medium text-muted-foreground">Password</label>
        <div class="flex gap-2">
          <div class="relative flex-1">
            <input
              class="w-full h-10 px-3 pr-10 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
              type={newShowPw ? 'text' : 'password'}
              bind:value={newPassword}
            />
            <button
              class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors text-xs"
              type="button"
              on:click={() => (newShowPw = !newShowPw)}
            >{newShowPw ? '🙈' : '👁️'}</button>
          </div>
          <button
            class="h-10 w-10 rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex items-center justify-center text-base"
            type="button"
            title="Generate password"
            on:click={() => (newPassword = generatePassword())}
          >↻</button>
        </div>
        <!-- Strength bar -->
        {#if newPassword}
          {@const s = pwStrength(newPassword)}
          <div class="flex gap-1 mt-1">
            {#each [1, 2, 3, 4] as seg}
              <div class="h-1 flex-1 rounded-full {seg <= s ? strengthColor[s] : 'bg-muted'}"></div>
            {/each}
          </div>
          <p class="text-xs text-muted-foreground">{strengthLabel[s]}</p>
        {/if}
      </div>

      <!-- Quota -->
      <div class="space-y-1">
        <label class="text-xs font-medium text-muted-foreground">Quota (MB) — 0 = unlimited</label>
        <input
          class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          type="number"
          min="0"
          bind:value={newQuota}
        />
      </div>

      <div class="flex gap-3 pt-1">
        <button
          class="flex-1 h-9 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={() => (showCreateAccount = false)}
        >
          Cancel
        </button>
        <button
          class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
          disabled={creatingAccount || !newLocalPart.trim() || !newPassword}
          on:click={submitCreateAccount}
        >
          {creatingAccount ? 'Creating…' : 'Create Account'}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── MODAL: Change Password ──────────────────────────────────────────────── -->
{#if showChangePw && changePwAccount}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    data-backdrop="1"
    on:click={(e) => closeIfBackdrop(e, () => (showChangePw = false))}
  >
    <div class="bg-card border border-border rounded-xl w-full max-w-md mx-4 p-6 space-y-5 shadow-2xl">
      <div class="flex items-center justify-between">
        <div>
          <h3 class="text-base font-semibold text-foreground">Change Password</h3>
          <p class="text-xs text-muted-foreground mt-0.5 font-mono">{emailAddress(changePwAccount)}</p>
        </div>
        <button
          class="h-7 w-7 rounded-md text-muted-foreground hover:bg-muted transition-colors flex items-center justify-center text-lg"
          on:click={() => (showChangePw = false)}
        >×</button>
      </div>

      <div class="space-y-1">
        <label class="text-xs font-medium text-muted-foreground">New Password</label>
        <div class="flex gap-2">
          <div class="relative flex-1">
            <input
              class="w-full h-10 px-3 pr-10 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
              type={changePwShow ? 'text' : 'password'}
              bind:value={changePwValue}
            />
            <button
              class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground transition-colors text-xs"
              type="button"
              on:click={() => (changePwShow = !changePwShow)}
            >{changePwShow ? '🙈' : '👁️'}</button>
          </div>
          <button
            class="h-10 w-10 rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex items-center justify-center text-base"
            type="button"
            title="Generate password"
            on:click={() => (changePwValue = generatePassword())}
          >↻</button>
        </div>
        {#if changePwValue}
          {@const s = pwStrength(changePwValue)}
          <div class="flex gap-1 mt-1">
            {#each [1, 2, 3, 4] as seg}
              <div class="h-1 flex-1 rounded-full {seg <= s ? strengthColor[s] : 'bg-muted'}"></div>
            {/each}
          </div>
          <p class="text-xs text-muted-foreground">{strengthLabel[s]}</p>
        {/if}
      </div>

      <div class="flex gap-3 pt-1">
        <button
          class="flex-1 h-9 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={() => (showChangePw = false)}
        >
          Cancel
        </button>
        <button
          class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
          disabled={changingPw || !changePwValue}
          on:click={submitChangePw}
        >
          {changingPw ? 'Saving…' : 'Update Password'}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── MODAL: Create Forwarder ─────────────────────────────────────────────── -->
{#if showCreateFwd}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    data-backdrop="1"
    on:click={(e) => closeIfBackdrop(e, () => (showCreateFwd = false))}
  >
    <div class="bg-card border border-border rounded-xl w-full max-w-md mx-4 p-6 space-y-5 shadow-2xl">
      <div class="flex items-center justify-between">
        <h3 class="text-base font-semibold text-foreground">New Forwarder</h3>
        <button
          class="h-7 w-7 rounded-md text-muted-foreground hover:bg-muted transition-colors flex items-center justify-center text-lg"
          on:click={() => (showCreateFwd = false)}
        >×</button>
      </div>

      <div class="space-y-1">
        <label class="text-xs font-medium text-muted-foreground">Source</label>
        <div class="flex items-center gap-0">
          <input
            class="flex-1 h-10 px-3 rounded-l-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
            type="text"
            placeholder="alias or @{site.domain} for catch-all"
            bind:value={fwdSource}
          />
          <span class="h-10 px-3 flex items-center bg-muted border border-l-0 border-border rounded-r-lg text-sm text-muted-foreground whitespace-nowrap">
            @{site.domain}
          </span>
        </div>
        <p class="text-xs text-muted-foreground">Use <span class="font-mono">@{site.domain}</span> as source for a catch-all forwarder</p>
      </div>

      <div class="space-y-1">
        <label class="text-xs font-medium text-muted-foreground">Destination</label>
        <input
          class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          type="email"
          placeholder="destination@example.com"
          bind:value={fwdDest}
        />
      </div>

      <div class="flex gap-3 pt-1">
        <button
          class="flex-1 h-9 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={() => (showCreateFwd = false)}
        >
          Cancel
        </button>
        <button
          class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
          disabled={creatingFwd || !fwdSource.trim() || !fwdDest.trim()}
          on:click={submitCreateFwd}
        >
          {creatingFwd ? 'Creating…' : 'Create Forwarder'}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Delete account confirmation ──────────────────────────────────────────── -->
{#if deleteAccountTarget}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm" on:click|self={closeDeleteAccount}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl">
      <div class="flex items-center gap-3 mb-3">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7"/>
          </svg>
        </div>
        <h3 class="text-base font-semibold text-foreground">Delete email account</h3>
      </div>
      <p class="text-sm text-muted-foreground mb-4">
        This permanently deletes <strong class="text-foreground font-mono">{emailAddress(deleteAccountTarget)}</strong> and all stored mail.
        This action <strong class="text-red-400">cannot be undone</strong>.
      </p>
      <label class="block text-xs text-muted-foreground mb-1.5">
        Type the full address to confirm:
      </label>
      <input
        bind:value={deleteAccountConfirm}
        type="text"
        autocomplete="off"
        class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground font-mono focus:outline-none focus:ring-2 focus:ring-red-500/50"
        placeholder={emailAddress(deleteAccountTarget)}
      />
      <div class="flex items-center gap-2 justify-end mt-5">
        <button on:click={closeDeleteAccount} class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors">
          Cancel
        </button>
        <button
          on:click={confirmDeleteAccount}
          disabled={deleteAccountConfirm !== emailAddress(deleteAccountTarget) || deletingAccountId === deleteAccountTarget.id}
          class="h-9 px-4 rounded-lg bg-red-500 text-white text-sm font-semibold hover:bg-red-600 transition-colors disabled:opacity-40 disabled:cursor-not-allowed inline-flex items-center gap-2"
        >
          {#if deletingAccountId === deleteAccountTarget.id}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
            Deleting…
          {:else}
            Delete account
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}
