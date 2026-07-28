<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { auth } from '$lib/stores/auth';
  import type { Site } from '$lib/api/client';

  // ── Props ──────────────────────────────────────────────────────────────────
  export let siteId: string;
  export let site: Site;

  // ── Local types ────────────────────────────────────────────────────────────
  interface SshKey {
    id: string;
    label: string;
    public_key: string;
    fingerprint: string;
    created_at: string;
  }

  interface FtpUser {
    id: string;
    username: string;
    home_dir: string;
    created_at: string;
  }

  interface Toast {
    id: number;
    message: string;
    type: 'success' | 'error';
  }

  // ── Auth helper ────────────────────────────────────────────────────────────
  function authH(): Record<string, string> {
    const t = get(auth).token;
    return t
      ? { 'Authorization': 'Bearer ' + t, 'Content-Type': 'application/json' }
      : { 'Content-Type': 'application/json' };
  }

  // ── Server IP derivation ───────────────────────────────────────────────────
  function deriveServerIp(): string {
    if (site.server_label) {
      const m = site.server_label.match(/\b(\d{1,3}(?:\.\d{1,3}){3})\b/);
      if (m) return m[1];
    }
    return '192.168.0.79';
  }

  $: serverIp = deriveServerIp();
  $: sftpCommand = `sftp -P 22 ${site.unix_user}@${serverIp}`;
  $: rootDir = `/home/${site.unix_user}/public_html`;
  $: jailDir = `/home/${site.unix_user}/`;

  // ── State ──────────────────────────────────────────────────────────────────
  let sshKeys: SshKey[] = [];
  let ftpUsers: FtpUser[] = [];
  let sshLoading = true;
  let ftpLoading = true;
  let sshApiAvailable = true;
  let ftpApiAvailable = true;

  // SSH Key modal
  let showAddKey = false;
  let newKeyLabel = '';
  let newKeyValue = '';
  let keyAdding = false;
  let keyError = '';

  // FTP Create modal
  let showCreateFtp = false;
  let newFtpUser = '';
  let newFtpPassword = '';
  let showNewFtpPw = false;
  let ftpCreating = false;
  let ftpCreateError = '';

  // FTP Change Password modal
  let showFtpPwChange: FtpUser | null = null;
  let ftpNewPassword = '';
  let showFtpNewPw = false;
  let ftpPwChanging = false;

  // Inline confirm state
  let confirmDeleteKey: SshKey | null = null;
  let confirmDeleteFtp: FtpUser | null = null;

  // SFTP management
  let sftpConfiguring = false;
  let sftpConfigured = false;
  let showSftpPwForm = false;
  let sftpPassword = '';
  let showSftpPw = false;
  let sftpPwSetting = false;

  // Toast
  let toasts: Toast[] = [];
  let toastCounter = 0;

  // ── Helpers ────────────────────────────────────────────────────────────────
  function generatePassword(len = 16): string {
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

  function showToast(message: string, type: 'success' | 'error' = 'success') {
    const id = ++toastCounter;
    toasts = [...toasts, { id, message, type }];
    setTimeout(() => { toasts = toasts.filter(t => t.id !== id); }, 4000);
  }

  async function copyToClipboard(text: string, label: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast(`${label} copied to clipboard`);
    } catch {
      showToast('Copy failed', 'error');
    }
  }

  function keyType(pub: string): string {
    if (pub.startsWith('ssh-ed25519')) return 'ED25519';
    if (pub.startsWith('ssh-rsa')) return 'RSA';
    if (pub.startsWith('ecdsa-sha2-nistp256')) return 'ECDSA';
    return 'Unknown';
  }

  function relativeDate(iso: string): string {
    const diff = Date.now() - new Date(iso).getTime();
    const days = Math.floor(diff / 86400000);
    const hours = Math.floor(diff / 3600000);
    const mins = Math.floor(diff / 60000);
    if (days > 30) return new Date(iso).toLocaleDateString();
    if (days >= 1) return `${days} day${days !== 1 ? 's' : ''} ago`;
    if (hours >= 1) return `${hours} hour${hours !== 1 ? 's' : ''} ago`;
    if (mins >= 1) return `${mins} min${mins !== 1 ? 's' : ''} ago`;
    return 'just now';
  }

  function closeOnEsc(e: KeyboardEvent) {
    if (e.key === 'Escape') closeAllModals();
  }

  function closeAllModals() {
    showAddKey = false;
    showCreateFtp = false;
    showFtpPwChange = null;
    keyError = '';
    ftpCreateError = '';
  }

  // ── API loaders ────────────────────────────────────────────────────────────
  async function loadSshKeys() {
    sshLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/ssh-keys`, { headers: authH() });
      if (res.status === 404 || res.status === 501) { sshApiAvailable = false; return; }
      if (res.ok) sshKeys = await res.json() as SshKey[];
    } catch {
      sshApiAvailable = false;
    } finally {
      sshLoading = false;
    }
  }

  async function loadFtpUsers() {
    ftpLoading = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/ftp-users`, { headers: authH() });
      if (res.status === 404 || res.status === 501) { ftpApiAvailable = false; return; }
      if (res.ok) ftpUsers = await res.json() as FtpUser[];
    } catch {
      ftpApiAvailable = false;
    } finally {
      ftpLoading = false;
    }
  }

  // ── SSH Key actions ────────────────────────────────────────────────────────
  function openAddKey() {
    newKeyLabel = '';
    newKeyValue = '';
    keyError = '';
    showAddKey = true;
  }

  async function handleAddKey() {
    keyError = '';
    if (!newKeyLabel.trim()) {
      keyError = 'Label is required';
      return;
    }
    const trimmed = newKeyValue.trim();
    if (!trimmed) {
      keyError = 'Public key is required';
      return;
    }
    if (!trimmed.startsWith('ssh-') && !trimmed.startsWith('ecdsa-')) {
      keyError = 'Public key must start with ssh- or ecdsa-';
      return;
    }
    keyAdding = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/ssh-keys`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ label: newKeyLabel.trim(), public_key: trimmed }),
      });
      if (!res.ok) {
        const err = await res.json() as { message?: string };
        throw new Error(err.message ?? `HTTP ${res.status}`);
      }
      const created = await res.json() as SshKey;
      sshKeys = [...sshKeys, created];
      showAddKey = false;
      showToast('SSH key added successfully');
    } catch (err: unknown) {
      keyError = (err as { message?: string }).message ?? 'Failed to add key';
    } finally {
      keyAdding = false;
    }
  }

  async function deleteKey(key: SshKey) {
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/ssh-keys/${key.id}`, {
        method: 'DELETE',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      sshKeys = sshKeys.filter(k => k.id !== key.id);
      confirmDeleteKey = null;
      showToast('SSH key deleted');
    } catch {
      showToast('Failed to delete SSH key', 'error');
    }
  }

  // ── FTP User actions ───────────────────────────────────────────────────────
  function openCreateFtp() {
    newFtpUser = '';
    newFtpPassword = generatePassword();
    showNewFtpPw = false;
    ftpCreateError = '';
    showCreateFtp = true;
  }

  async function handleCreateFtp() {
    ftpCreateError = '';
    ftpCreating = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/ftp-users`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ username: newFtpUser.trim(), password: newFtpPassword }),
      });
      if (!res.ok) {
        const err = await res.json() as { message?: string };
        throw new Error(err.message ?? `HTTP ${res.status}`);
      }
      const created = await res.json() as FtpUser;
      ftpUsers = [...ftpUsers, created];
      showCreateFtp = false;
      showToast('FTP user created successfully');
    } catch (err: unknown) {
      ftpCreateError = (err as { message?: string }).message ?? 'Failed to create FTP user';
    } finally {
      ftpCreating = false;
    }
  }

  function openFtpPwChange(user: FtpUser) {
    ftpNewPassword = generatePassword();
    showFtpNewPw = false;
    showFtpPwChange = user;
  }

  async function handleFtpPwChange() {
    if (!showFtpPwChange) return;
    ftpPwChanging = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/ftp-users/${showFtpPwChange.id}/password`, {
        method: 'PUT',
        headers: authH(),
        body: JSON.stringify({ password: ftpNewPassword }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      showFtpPwChange = null;
      showToast('FTP password changed successfully');
    } catch {
      showToast('Failed to change FTP password', 'error');
    } finally {
      ftpPwChanging = false;
    }
  }

  async function deleteFtpUser(user: FtpUser) {
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/ftp-users/${user.id}`, {
        method: 'DELETE',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      ftpUsers = ftpUsers.filter(u => u.id !== user.id);
      confirmDeleteFtp = null;
      showToast('FTP user deleted');
    } catch {
      showToast('Failed to delete FTP user', 'error');
    }
  }

  // ── SFTP actions ───────────────────────────────────────────────────────────
  async function configureSftp() {
    sftpConfiguring = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/sftp/configure`, {
        method: 'POST',
        headers: authH(),
      });
      if (!res.ok) {
        const err = await res.json() as { message?: string };
        throw new Error(err.message ?? `HTTP ${res.status}`);
      }
      sftpConfigured = true;
      showToast('SFTP chroot configured successfully');
    } catch (err: unknown) {
      showToast((err as { message?: string }).message ?? 'Failed to configure SFTP', 'error');
    } finally {
      sftpConfiguring = false;
    }
  }

  async function setSftpPassword() {
    if (sftpPassword.length < 8) {
      showToast('Password must be at least 8 characters', 'error');
      return;
    }
    sftpPwSetting = true;
    try {
      const res = await fetch(`/api/v1/sites/${siteId}/sftp/set-password`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ password: sftpPassword }),
      });
      if (!res.ok) {
        const err = await res.json() as { message?: string };
        throw new Error(err.message ?? `HTTP ${res.status}`);
      }
      showSftpPwForm = false;
      sftpPassword = '';
      showToast('SFTP password set successfully');
    } catch (err: unknown) {
      showToast((err as { message?: string }).message ?? 'Failed to set SFTP password', 'error');
    } finally {
      sftpPwSetting = false;
    }
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(() => {
    void loadSshKeys();
    void loadFtpUsers();
  });

  // ── Reactive ───────────────────────────────────────────────────────────────
  $: ftpPwStrength = pwStrength(ftpNewPassword);
  $: newFtpPwStrength = pwStrength(newFtpPassword);
  $: sftpPwStrength = pwStrength(sftpPassword);
</script>

<svelte:window on:keydown={closeOnEsc} />

<div class="tab-content space-y-6">

  <!-- ── Section 1: SFTP Connection Info ─────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-5">
    <div class="flex items-center justify-between mb-4">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6.75 7.5l3 2.25-3 2.25m4.5 0h3m-9 8.25h13.5A2.25 2.25 0 0021 18V6a2.25 2.25 0 00-2.25-2.25H5.25A2.25 2.25 0 003 6v12a2.25 2.25 0 002.25 2.25z"/>
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">SFTP Connection Details</h2>
      </div>
      <!-- SFTP Chroot status badge -->
      <span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border
        {sftpConfigured
          ? 'bg-green-500/10 text-green-400 border-green-500/20'
          : 'bg-muted text-muted-foreground border-border'}">
        <span class="w-1.5 h-1.5 rounded-full {sftpConfigured ? 'bg-green-400' : 'bg-muted-foreground'}"></span>
        {sftpConfigured ? 'Chroot Active' : 'Chroot Not Configured'}
      </span>
    </div>

    <!-- Info grid -->
    <div class="space-y-2 mb-4">
      {#each [
        { label: 'Host',           value: serverIp },
        { label: 'Port',           value: '22' },
        { label: 'Username',       value: site.unix_user },
        { label: 'Root directory', value: rootDir },
        { label: 'Auth method',    value: 'SSH key (preferred) or password' },
      ] as row}
        <div class="flex items-center justify-between py-2 px-3 rounded-lg bg-muted/30">
          <div class="flex items-center gap-3 min-w-0">
            <span class="text-xs text-muted-foreground w-32 flex-shrink-0">{row.label}</span>
            <span class="font-mono text-sm text-foreground truncate">{row.value}</span>
          </div>
          {#if row.label !== 'Auth method'}
            <button
              class="h-7 px-2 text-xs rounded border border-border text-muted-foreground hover:bg-muted transition-colors flex-shrink-0 ml-2"
              on:click={() => copyToClipboard(row.value, row.label)}
            >
              Copy
            </button>
          {/if}
        </div>
      {/each}
    </div>

    <!-- Connection string snippet -->
    <div class="mb-4">
      <div class="flex items-center justify-between mb-1.5">
        <span class="text-xs font-medium text-muted-foreground">Connection command</span>
        <button
          class="h-6 px-2 text-xs rounded border border-border text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-1"
          on:click={() => copyToClipboard(sftpCommand, 'Connection command')}
        >
          <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"/>
          </svg>
          Copy
        </button>
      </div>
      <div class="bg-[#0d1117] border border-border/40 rounded-lg px-4 py-3 font-mono text-xs text-green-400 break-all flex items-center gap-2">
        <span class="text-slate-500 select-none">$</span>
        <span>{sftpCommand}</span>
      </div>
    </div>

    <!-- Jail note -->
    <div class="bg-blue-500/10 border border-blue-500/20 rounded-xl p-4 text-sm text-blue-300 mb-4">
      <div class="flex gap-2 items-start">
        <span class="mt-0.5 flex-shrink-0">ℹ️</span>
        <p>SFTP access is restricted to <code class="font-mono text-xs bg-blue-500/20 px-1 rounded">{jailDir}</code> via chroot. Upload files to <code class="font-mono text-xs bg-blue-500/20 px-1 rounded">public_html/</code>.</p>
      </div>
    </div>

    <!-- SFTP Actions -->
    <div class="flex flex-wrap gap-2">
      <button
        class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-2 disabled:opacity-50"
        disabled={sftpConfiguring}
        on:click={configureSftp}
      >
        {#if sftpConfiguring}
          <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
            <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"></path>
          </svg>
          Configuring...
        {:else if sftpConfigured}
          <svg class="w-3.5 h-3.5 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
          </svg>
          Chroot Active
        {:else}
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
            <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          </svg>
          Configure SFTP Chroot
        {/if}
      </button>
      <button
        class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors inline-flex items-center gap-2"
        on:click={() => { showSftpPwForm = !showSftpPwForm; sftpPassword = generatePassword(); }}
      >
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
        </svg>
        Set SFTP Password
      </button>
    </div>

    {#if showSftpPwForm}
      <div class="mt-4 p-4 bg-muted/20 border border-border rounded-xl space-y-3">
        <p class="text-xs text-muted-foreground">Set a password for SFTP access (alternative to SSH key auth)</p>
        <div class="flex gap-2">
          <div class="relative flex-1">
            <input
              type={showSftpPw ? 'text' : 'password'}
              bind:value={sftpPassword}
              class="w-full h-10 px-3 pr-9 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
            />
            <button
              type="button"
              class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              on:click={() => { showSftpPw = !showSftpPw; }}
            >
              <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                {#if showSftpPw}
                  <path stroke-linecap="round" stroke-linejoin="round" d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                {:else}
                  <path stroke-linecap="round" stroke-linejoin="round" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                  <path stroke-linecap="round" stroke-linejoin="round" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                {/if}
              </svg>
            </button>
          </div>
          <button
            type="button"
            class="h-10 w-10 flex items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex-shrink-0"
            title="Generate password"
            on:click={() => { sftpPassword = generatePassword(); }}
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
          </button>
          <button
            class="h-10 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50 flex-shrink-0"
            disabled={sftpPwSetting || sftpPassword.length < 8}
            on:click={setSftpPassword}
          >
            {sftpPwSetting ? 'Saving…' : 'Set Password'}
          </button>
        </div>
      </div>
    {/if}
  </div>


  <!-- ── Section 2: SSH Keys ──────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-5">
    <!-- Header -->
    <div class="flex items-center justify-between mb-4">
      <div class="flex items-center gap-2.5">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 5.25a3 3 0 013 3m3 0a6 6 0 01-7.029 5.912c-.563-.097-1.159.026-1.563.43L10.5 17.25H8.25v2.25H6v2.25H2.25v-2.818c0-.597.237-1.17.659-1.591l6.499-6.499c.404-.404.527-1 .43-1.563A6 6 0 1121.75 8.25z"/>
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">SSH Keys</h2>
        {#if !sshLoading && sshApiAvailable}
          <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-muted text-muted-foreground">
            {sshKeys.length}
          </span>
        {/if}
      </div>
      {#if sshApiAvailable}
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-all active:scale-95 inline-flex items-center gap-1.5"
          on:click={openAddKey}
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15"/>
          </svg>
          Add SSH Key
        </button>
      {/if}
    </div>

    <!-- Coming soon banner -->
    {#if !sshApiAvailable}
      <div class="bg-blue-500/10 border border-blue-500/20 rounded-xl p-4 text-sm text-blue-300">
        <div class="flex gap-2 items-start">
          <span class="mt-0.5">ℹ️</span>
          <div>
            <p class="font-medium">SSH key management will be available soon.</p>
            <p class="text-xs text-blue-400 mt-1">You can still connect via SFTP using the credentials shown above.</p>
          </div>
        </div>
      </div>

    <!-- Loading -->
    {:else if sshLoading}
      <div class="space-y-2">
        {#each [1, 2] as _}
          <div class="animate-pulse bg-muted rounded-xl h-16"></div>
        {/each}
      </div>

    <!-- Empty state -->
    {:else if sshKeys.length === 0}
      <div class="text-center py-10">
        <svg class="mx-auto mb-3 w-10 h-10 text-muted-foreground/40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
        </svg>
        <p class="text-muted-foreground font-medium mb-1">No SSH keys added</p>
        <p class="text-muted-foreground/60 text-sm mb-4">Add a public key for password-free SFTP access.</p>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
          on:click={openAddKey}
        >
          Add your first key
        </button>
      </div>

    <!-- Key list -->
    {:else}
      <div class="space-y-2">
        {#each sshKeys as key (key.id)}
          <div class="card-hover flex items-center justify-between py-3 px-4 rounded-xl border border-border bg-background">
            <div class="min-w-0 flex-1 mr-4">
              <div class="flex items-center gap-2 mb-1.5">
                <span class="font-medium text-sm text-foreground">{key.label}</span>
                <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-primary/10 text-primary border border-primary/20">
                  {keyType(key.public_key)}
                </span>
              </div>
              <p class="bg-muted px-2 py-0.5 rounded font-mono text-xs text-muted-foreground truncate inline-block max-w-full">{key.fingerprint}</p>
              <p class="text-xs text-muted-foreground/60 mt-1">Added {relativeDate(key.created_at)}</p>
            </div>
            {#if confirmDeleteKey?.id === key.id}
              <div class="flex items-center gap-1.5 shrink-0">
                <span class="text-xs text-destructive">Delete?</span>
                <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => deleteKey(key)}>Yes</button>
                <button class="text-xs px-2 py-1 rounded bg-muted text-foreground hover:bg-muted/80" on:click={() => confirmDeleteKey = null}>No</button>
              </div>
            {:else}
              <button
                title="Delete key"
                class="text-red-400 hover:bg-red-500/10 px-3 py-1.5 rounded-lg text-sm transition-colors flex-shrink-0"
                on:click={() => confirmDeleteKey = key}
              >
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                </svg>
              </button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </div>


  <!-- ── Section 3: FTP Users ─────────────────────────────────────────────────── -->
  <div class="bg-card border border-border rounded-xl p-5">
    <!-- Header -->
    <div class="flex items-center justify-between mb-1">
      <div class="flex items-center gap-2.5 flex-wrap">
        <div class="w-7 h-7 rounded-lg bg-primary/10 flex items-center justify-center shrink-0">
          <svg class="w-3.5 h-3.5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M15.75 6a3.75 3.75 0 11-7.5 0 3.75 3.75 0 017.5 0zM4.501 20.118a7.5 7.5 0 0114.998 0A17.933 17.933 0 0112 21.75c-2.676 0-5.216-.584-7.499-1.632z"/>
          </svg>
        </div>
        <h2 class="text-sm font-semibold text-foreground">FTP Users</h2>
        <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-amber-500/10 text-amber-400 border border-amber-500/20">
          Restricted to site directory
        </span>
      </div>
      {#if ftpApiAvailable}
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-all active:scale-95 inline-flex items-center gap-1.5"
          on:click={openCreateFtp}
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M12 4.5v15m7.5-7.5h-15"/>
          </svg>
          New FTP User
        </button>
      {/if}
    </div>
    <p class="text-xs text-muted-foreground mb-4">FileZilla / WinSCP compatible — restricted to <code class="font-mono bg-muted px-1 rounded">{jailDir}</code></p>

    <!-- Coming soon banner -->
    {#if !ftpApiAvailable}
      <div class="bg-blue-500/10 border border-blue-500/20 rounded-xl p-4 text-sm text-blue-300">
        <div class="flex gap-2 items-start">
          <span class="mt-0.5">ℹ️</span>
          <div>
            <p class="font-medium">FTP user management will be available soon.</p>
            <p class="text-xs text-blue-400 mt-1">You can still connect via SFTP using the credentials shown above.</p>
          </div>
        </div>
      </div>

    <!-- Loading -->
    {:else if ftpLoading}
      <div class="space-y-2">
        {#each [1, 2] as _}
          <div class="animate-pulse bg-muted rounded h-10"></div>
        {/each}
      </div>

    <!-- User table -->
    {:else if ftpUsers.length > 0}
      <div class="bg-card border border-border rounded-xl overflow-hidden mb-4">
        <table class="w-full text-sm">
          <thead>
            <tr class="bg-muted/30">
              <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Username</th>
              <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Home Dir</th>
              <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Created</th>
              <th class="text-right px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Actions</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-border">
            {#each ftpUsers as user (user.id)}
              <tr class="hover:bg-muted/10 transition-colors">
                <td class="px-4 py-3 font-mono text-xs text-foreground">{user.username}</td>
                <td class="px-4 py-3 font-mono text-xs text-muted-foreground max-w-[200px]">
                  <span class="truncate block">{user.home_dir}</span>
                </td>
                <td class="px-4 py-3 text-xs text-muted-foreground">{relativeDate(user.created_at)}</td>
                <td class="px-4 py-3">
                  <div class="flex items-center justify-end gap-1">
                    <button
                      title="Change password"
                      class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors"
                      on:click={() => openFtpPwChange(user)}
                    >
                      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                          d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                      </svg>
                    </button>
                    {#if confirmDeleteFtp?.id === user.id}
                      <div class="flex items-center gap-1">
                        <button class="text-xs px-2 py-0.5 rounded bg-destructive text-white hover:bg-destructive/90" on:click={() => deleteFtpUser(user)}>Delete</button>
                        <button class="text-xs px-2 py-0.5 rounded bg-muted text-foreground hover:bg-muted/80" on:click={() => confirmDeleteFtp = null}>Cancel</button>
                      </div>
                    {:else}
                      <button
                        title="Delete user"
                        class="h-8 w-8 flex items-center justify-center rounded-lg text-red-400 hover:bg-red-500/10 transition-colors"
                        on:click={() => confirmDeleteFtp = user}
                      >
                        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                        </svg>
                      </button>
                    {/if}
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

    <!-- Empty state -->
    {:else}
      <div class="text-center py-8 mb-4">
        <svg class="mx-auto mb-3 w-10 h-10 text-muted-foreground/40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
        </svg>
        <p class="text-muted-foreground font-medium mb-1">No FTP users yet</p>
        <p class="text-muted-foreground/60 text-sm mb-4">Create an FTP account to connect with FileZilla or WinSCP.</p>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
          on:click={openCreateFtp}
        >
          Create your first FTP user
        </button>
      </div>
    {/if}

    <!-- FTP Connection Info card (always shown when API available) -->
    {#if ftpApiAvailable}
      <div class="bg-muted/20 border border-border rounded-xl p-4 mt-2">
        <h3 class="text-sm font-semibold text-foreground mb-3">FTP Connection Info</h3>
        <div class="space-y-1.5 mb-3 text-sm">
          {#each [
            { label: 'Protocol', value: 'FTP / FTPS (Explicit TLS)' },
            { label: 'Host',     value: serverIp },
            { label: 'Port',     value: '21' },
            { label: 'Path',     value: '/' },
            { label: 'Auth',     value: 'Your FTP username and password' },
          ] as row}
            <div class="flex items-center gap-3">
              <span class="text-xs text-muted-foreground w-20 flex-shrink-0">{row.label}</span>
              <span class="font-mono text-xs text-foreground">{row.value}</span>
              {#if row.label === 'Host'}
                <button
                  class="h-6 px-2 text-xs rounded border border-border text-muted-foreground hover:bg-muted transition-colors ml-auto"
                  on:click={() => copyToClipboard(row.value, 'Host')}
                >
                  Copy
                </button>
              {/if}
            </div>
          {/each}
        </div>
        <div class="bg-blue-500/10 border border-blue-500/20 rounded-lg p-3 text-xs text-blue-300">
          <span class="font-medium">FileZilla:</span> File → Site Manager → New Site → Protocol: FTPS → Host: <span class="font-mono">{serverIp}</span> → Port: 21 → Encryption: Require explicit FTP over TLS
        </div>
      </div>
    {/if}
  </div>
</div>


<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
  .card-hover { transition: all 0.2s ease; }
  .card-hover:hover { transform: translateY(-1px); box-shadow: 0 8px 30px rgba(0,0,0,0.2); }
</style>

<!-- ── Add SSH Key Modal ──────────────────────────────────────────────────────── -->
{#if showAddKey}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeAllModals}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Add SSH Key</h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={closeAllModals}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="mb-4">
        <label class="block text-xs font-medium text-muted-foreground mb-1">Label</label>
        <input
          type="text"
          bind:value={newKeyLabel}
          placeholder="My MacBook"
          class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        />
      </div>

      <div class="mb-5">
        <label class="block text-xs font-medium text-muted-foreground mb-1">Public Key</label>
        <textarea
          bind:value={newKeyValue}
          rows={4}
          placeholder="ssh-ed25519 AAAA... user@host"
          class="w-full px-3 py-2 rounded-lg border border-border bg-background text-sm text-foreground font-mono resize-none focus:outline-none focus:ring-2 focus:ring-ring"
        ></textarea>
        <p class="text-xs text-muted-foreground mt-1">Paste the contents of your <code class="font-mono">~/.ssh/id_ed25519.pub</code> or similar file.</p>
      </div>

      {#if keyError}
        <p class="text-sm text-red-400 mb-3">{keyError}</p>
      {/if}

      <div class="flex justify-end gap-2">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={closeAllModals}
        >
          Cancel
        </button>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
          disabled={keyAdding || !newKeyLabel.trim() || !newKeyValue.trim()}
          on:click={handleAddKey}
        >
          {keyAdding ? 'Adding…' : 'Add Key'}
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Create FTP User Modal ──────────────────────────────────────────────────── -->
{#if showCreateFtp}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeAllModals}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">New FTP User</h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={closeAllModals}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <!-- Username -->
      <div class="mb-4">
        <label class="block text-xs font-medium text-muted-foreground mb-1">Username</label>
        <input
          type="text"
          bind:value={newFtpUser}
          placeholder="myuser"
          class="w-full h-10 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
        />
        {#if newFtpUser.trim()}
          <p class="text-xs text-muted-foreground mt-1">Will be created as: <span class="font-mono text-foreground">{site.unix_user}_{newFtpUser.trim()}</span></p>
        {:else}
          <p class="text-xs text-muted-foreground mt-1">Will be prefixed as <span class="font-mono">{site.unix_user}_username</span></p>
        {/if}
      </div>

      <!-- Password -->
      <div class="mb-4">
        <label class="block text-xs font-medium text-muted-foreground mb-1">Password</label>
        <div class="flex gap-2">
          <div class="relative flex-1">
            <input
              type={showNewFtpPw ? 'text' : 'password'}
              bind:value={newFtpPassword}
              class="w-full h-10 px-3 pr-9 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
            />
            <button
              type="button"
              class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              on:click={() => { showNewFtpPw = !showNewFtpPw; }}
            >
              {#if showNewFtpPw}
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                </svg>
              {:else}
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                </svg>
              {/if}
            </button>
          </div>
          <button
            type="button"
            class="h-10 w-10 flex items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex-shrink-0"
            title="Generate password"
            on:click={() => { newFtpPassword = generatePassword(); }}
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
          </button>
        </div>
        <!-- Strength bar -->
        <div class="flex gap-1 mt-2">
          {#each [1, 2, 3, 4] as seg}
            <div class="flex-1 h-1 rounded-full transition-colors {newFtpPwStrength >= seg
              ? seg <= 1 ? 'bg-red-400' : seg <= 2 ? 'bg-orange-400' : seg <= 3 ? 'bg-yellow-400' : 'bg-green-400'
              : 'bg-muted'}"></div>
          {/each}
        </div>
      </div>

      <!-- Home dir (readonly) -->
      <div class="mb-5">
        <label class="block text-xs font-medium text-muted-foreground mb-1">Home Directory</label>
        <input
          type="text"
          value={jailDir}
          readonly
          class="w-full h-10 px-3 rounded-lg border border-border bg-muted/30 text-sm text-muted-foreground font-mono cursor-not-allowed"
        />
      </div>

      {#if ftpCreateError}
        <p class="text-sm text-red-400 mb-3">{ftpCreateError}</p>
      {/if}

      <div class="flex justify-end gap-2">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={closeAllModals}
        >
          Cancel
        </button>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
          disabled={ftpCreating || !newFtpUser.trim() || newFtpPassword.length < 8}
          on:click={handleCreateFtp}
        >
          {ftpCreating ? 'Creating…' : 'Create FTP User'}
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Change FTP Password Modal ──────────────────────────────────────────────── -->
{#if showFtpPwChange}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeAllModals}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Change FTP Password — <span class="font-mono">{showFtpPwChange.username}</span></h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={closeAllModals}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="mb-5">
        <label class="block text-xs font-medium text-muted-foreground mb-1">New Password</label>
        <div class="flex gap-2">
          <div class="relative flex-1">
            <input
              type={showFtpNewPw ? 'text' : 'password'}
              bind:value={ftpNewPassword}
              class="w-full h-10 px-3 pr-9 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
            />
            <button
              type="button"
              class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              on:click={() => { showFtpNewPw = !showFtpNewPw; }}
            >
              {#if showFtpNewPw}
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                </svg>
              {:else}
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                </svg>
              {/if}
            </button>
          </div>
          <button
            type="button"
            class="h-10 w-10 flex items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex-shrink-0"
            title="Generate password"
            on:click={() => { ftpNewPassword = generatePassword(); }}
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
          </button>
        </div>
        <!-- Strength bar -->
        <div class="flex gap-1 mt-2">
          {#each [1, 2, 3, 4] as seg}
            <div class="flex-1 h-1 rounded-full transition-colors {ftpPwStrength >= seg
              ? seg <= 1 ? 'bg-red-400' : seg <= 2 ? 'bg-orange-400' : seg <= 3 ? 'bg-yellow-400' : 'bg-green-400'
              : 'bg-muted'}"></div>
          {/each}
        </div>
        <p class="text-xs text-muted-foreground mt-1">
          Strength: {['', 'Weak', 'Fair', 'Good', 'Strong'][ftpPwStrength]}
        </p>
      </div>

      <div class="flex justify-end gap-2">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={closeAllModals}
        >
          Cancel
        </button>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
          disabled={ftpPwChanging || ftpNewPassword.length < 8}
          on:click={handleFtpPwChange}
        >
          {ftpPwChanging ? 'Saving…' : 'Change Password'}
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Toast notifications ────────────────────────────────────────────────────── -->
<div class="fixed bottom-5 right-5 z-[60] flex flex-col gap-2 pointer-events-none">
  {#each toasts as toast (toast.id)}
    <div
      class="pointer-events-auto flex items-center gap-3 px-4 py-3 rounded-xl shadow-lg border text-sm font-medium transition-all
        {toast.type === 'success'
          ? 'bg-card border-green-500/30 text-foreground'
          : 'bg-card border-red-500/30 text-foreground'}"
    >
      {#if toast.type === 'success'}
        <svg class="w-4 h-4 text-green-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7" />
        </svg>
      {:else}
        <svg class="w-4 h-4 text-red-400 flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 8v4m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
        </svg>
      {/if}
      <span>{toast.message}</span>
    </div>
  {/each}
</div>
