<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { api } from '$api/client';
  import type { User } from '$api/client';
  import { auth, currentUser } from '$stores/auth';
  import { t } from '$lib/i18n';

  // ── State ──────────────────────────────────────────────────────────────────
  let users: User[] = [];
  let loading = true;
  let searchQuery = '';
  let roleFilter: '' | 'admin' | 'reseller' | 'user' = '';

  // View toggle
  let viewMode: 'table' | 'cards' = 'table';

  // Activity log
  let showActivityLog = false;
  let activityLog: ActivityEvent[] = [];
  let activityLoading = false;

  // Permission matrix modal
  let showPermissionsModal = false;

  // Bulk selection
  let selectedIds = new Set<string>();
  let bulkLoading = false;

  // Invite modal
  let showInviteModal = false;
  let inviteForm = {
    email: '',
    full_name: '',
    role: 'user' as 'admin' | 'reseller' | 'user',
    send_email: true,
    temp_password: '',
  };
  let inviteLoading = false;
  let inviteError = '';
  let generatedPassword = '';
  let passwordCopied = false;

  // Edit modal
  let showEditModal = false;
  let editUser: User | null = null;
  let editForm = { full_name: '', email: '', role: 'user' as User['role'], new_password: '' };
  let editLoading = false;
  let editError = '';

  // Delete modal
  let showDeleteModal = false;
  let deleteTarget: User | null = null;
  let deleteConfirmEmail = '';
  let deleteLoading = false;

  // Impersonate
  let impersonatingId: string | null = null;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';
  let toastTimer: ReturnType<typeof setTimeout> | null = null;

  // Bulk action menu (legacy fallback, superseded by floating bar)
  let showBulkMenu = false;
  let confirmBulkDelete = false;

  // ── Activity log types ──────────────────────────────────────────────────────

  interface ActivityEvent {
    id: string | number;
    user_email: string;
    type: string;
    description: string;
    ip: string;
    created_at: string;
    success: boolean;
  }

  interface AuditLogEntry {
    id: number;
    action: string;
    resource_type?: string;
    resource_id?: string;
    ip_address?: string;
    created_at: string;
    user_email?: string;
  }

  function mapAuditEntry(e: AuditLogEntry): ActivityEvent {
    const actionMap: Record<string, string> = {
      'user.login': 'login', 'login': 'login',
      'user.logout': 'logout', 'logout': 'logout',
      'site.create': 'site_created', 'site_created': 'site_created',
      'site.delete': 'site_deleted', 'site_deleted': 'site_deleted',
      'user.password_change': 'password_changed', 'password_changed': 'password_changed',
      'email.create': 'email_created', 'email_created': 'email_created',
      'backup.create': 'backup_created', 'backup_created': 'backup_created',
      'database.create': 'db_created', 'db_created': 'db_created',
    };
    const descMap: Record<string, string> = {
      login: 'signed in', logout: 'signed out',
      site_created: 'created a site', site_deleted: 'deleted a site',
      password_changed: 'changed password', email_created: 'created an email account',
      backup_created: 'triggered a backup', db_created: 'created a database',
    };
    const type = actionMap[e.action] ?? e.action.replace(/\./g, '_');
    return {
      id: e.id,
      user_email: e.user_email ?? 'Unknown',
      type,
      description: descMap[type] ?? e.action,
      ip: e.ip_address ?? '—',
      created_at: e.created_at,
      success: true,
    };
  }

  onMount(async () => {
    await loadUsers();
  });

  async function loadUsers() {
    loading = true;
    loadError = '';
    try {
      users = await api.users.list();
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load users';
    } finally {
      loading = false;
    }
  }

  let activityError = '';

  async function loadActivityLog() {
    activityLoading = true;
    activityError = '';
    try {
      const token = typeof localStorage !== 'undefined' ? (localStorage.getItem('orbit_access_token') ?? '') : '';
      const res = await fetch('/api/v1/audit-log?limit=50', {
        headers: { 'Authorization': `Bearer ${token}` }
      });
      if (res.ok) {
        const raw: AuditLogEntry[] = await res.json();
        activityLog = raw.map(mapAuditEntry);
      } else {
        activityError = `Failed to load activity (HTTP ${res.status})`;
        activityLog = [];
      }
    } catch {
      activityError = 'Failed to load activity log';
      activityLog = [];
    } finally {
      activityLoading = false;
    }
  }

  async function toggleActivityLog() {
    showActivityLog = !showActivityLog;
    if (showActivityLog && activityLog.length === 0) {
      await loadActivityLog();
    }
  }

  // ── Permission matrix ───────────────────────────────────────────────────────

  type PermLevel = 'Full' | 'Own' | 'None' | 'Custom';

  interface UserPermissions {
    sites: PermLevel;
    dbs: PermLevel;
    email: PermLevel;
    dns: PermLevel;
    ssl: PermLevel;
    backups: PermLevel;
    logs: PermLevel;
    settings: PermLevel;
  }

  function defaultPerms(user: User): UserPermissions {
    if (user.role === 'admin') {
      return { sites: 'Full', dbs: 'Full', email: 'Full', dns: 'Full', ssl: 'Full', backups: 'Full', logs: 'Full', settings: 'Full' };
    }
    return { sites: 'Own', dbs: 'Own', email: 'Own', dns: 'Own', ssl: 'Own', backups: 'Own', logs: 'Own', settings: 'None' };
  }

  // In-memory perm overrides (persisted to API in a real integration)
  let permOverrides: Record<string, UserPermissions> = {};

  function cyclePermLevel(userId: string, col: keyof UserPermissions) {
    const u = users.find(u => u.id === userId);
    if (!u || u.role === 'admin') return;
    const cycle: PermLevel[] = ['Full', 'Own', 'None'];
    const cur = permOverrides[userId]?.[col] ?? defaultPerms(u)[col];
    const next = cycle[(cycle.indexOf(cur) + 1) % cycle.length];
    permOverrides = {
      ...permOverrides,
      [userId]: { ...(permOverrides[userId] ?? defaultPerms(u)), [col]: next },
    };
  }

  function permBadgeClass(level: PermLevel): string {
    const map: Record<PermLevel, string> = {
      Full:   'bg-emerald-500/15 text-emerald-400 border border-emerald-500/25',
      Own:    'bg-blue-500/15 text-blue-400 border border-blue-500/25',
      None:   'bg-muted text-muted-foreground border border-border',
      Custom: 'bg-amber-500/15 text-amber-400 border border-amber-500/25',
    };
    return map[level];
  }

  const permColumns: { key: keyof UserPermissions; label: string }[] = [
    { key: 'sites',    label: 'Sites'    },
    { key: 'dbs',      label: 'DBs'      },
    { key: 'email',    label: 'Email'    },
    { key: 'dns',      label: 'DNS'      },
    { key: 'ssl',      label: 'SSL'      },
    { key: 'backups',  label: 'Backups'  },
    { key: 'logs',     label: 'Logs'     },
    { key: 'settings', label: 'Settings' },
  ];

  // ── Derived ─────────────────────────────────────────────────────────────────

  $: filteredUsers = users.filter(u => {
    const q = searchQuery.toLowerCase();
    const matchSearch = !q ||
      u.email.toLowerCase().includes(q) ||
      (u.display_name ?? '').toLowerCase().includes(q);
    const matchRole = !roleFilter || u.role === roleFilter;
    return matchSearch && matchRole;
  });

  $: totalUsers     = users.length;
  $: totalAdmins    = users.filter(u => u.role === 'admin').length;
  $: totalResellers = users.filter(u => u.role === 'reseller').length;
  $: totalRegular   = users.filter(u => u.role === 'user').length;

  // ── Invite ─────────────────────────────────────────────────────────────────

  function openInviteModal() {
    inviteForm = { email: '', full_name: '', role: 'user', send_email: true, temp_password: '' };
    inviteError = '';
    generatedPassword = '';
    showInviteModal = true;
  }

  function generatePassword(): string {
    const chars = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789!@#$';
    return Array.from({ length: 16 }, () => chars[Math.floor(Math.random() * chars.length)]).join('');
  }

  $: if (!inviteForm.send_email && showInviteModal && !generatedPassword) {
    generatedPassword = generatePassword();
  }

  async function handleInvite(e: SubmitEvent) {
    e.preventDefault();
    inviteError = '';
    inviteLoading = true;
    try {
      const password = inviteForm.send_email
        ? (Math.random().toString(36) + Math.random().toString(36))
        : generatedPassword;
      const user = await api.users.create({
        email: inviteForm.email,
        role: inviteForm.role,
        password,
      });
      users = [...users, user];
      showInviteModal = false;
      showToast('User created successfully', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      inviteError = e.message ?? 'Failed to create user';
    } finally {
      inviteLoading = false;
    }
  }

  async function copyPassword() {
    await navigator.clipboard.writeText(generatedPassword);
    passwordCopied = true;
    setTimeout(() => { passwordCopied = false; }, 2000);
  }

  // ── Edit ───────────────────────────────────────────────────────────────────

  function openEditModal(user: User) {
    editUser = user;
    editForm = {
      full_name:    user.display_name ?? '',
      email:        user.email,
      role:         user.role,
      new_password: '',
    };
    editError = '';
    showEditModal = true;
  }

  async function handleEdit(e: SubmitEvent) {
    e.preventDefault();
    if (!editUser) return;
    editError = '';
    editLoading = true;
    try {
      if (editForm.new_password) {
        await api.users.changePassword(editUser.id, { new_password: editForm.new_password });
      }
      users = users.map(u => u.id === editUser!.id
        ? { ...u, display_name: editForm.full_name || null, email: editForm.email, role: editForm.role }
        : u
      );
      showEditModal = false;
      showToast('User updated', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      editError = e.message ?? 'Failed to update user';
    } finally {
      editLoading = false;
    }
  }

  // ── Delete ─────────────────────────────────────────────────────────────────

  function openDeleteModal(user: User) {
    deleteTarget = user;
    deleteConfirmEmail = '';
    showDeleteModal = true;
  }

  async function handleDelete() {
    if (!deleteTarget) return;
    if (deleteConfirmEmail !== deleteTarget.email) return;
    deleteLoading = true;
    try {
      await api.users.delete(deleteTarget.id);
      users = users.filter(u => u.id !== deleteTarget!.id);
      selectedIds.delete(deleteTarget.id);
      selectedIds = selectedIds;
      showDeleteModal = false;
      showToast('User deleted', 'success');
    } catch {
      showToast('Failed to delete user', 'error');
    } finally {
      deleteLoading = false;
      deleteTarget = null;
    }
  }

  // ── Suspend / Activate ─────────────────────────────────────────────────────

  async function toggleSuspend(user: User) {
    const u = user as User & { status?: string };
    const isSuspended = u.status === 'suspended';
    try {
      if (isSuspended) {
        await api.reseller.unsuspendClient(user.id);
      } else {
        await api.reseller.suspendClient(user.id);
      }
      users = users.map(x => x.id === user.id
        ? { ...x, status: isSuspended ? 'active' : 'suspended' } as User
        : x
      );
      showToast(isSuspended ? 'User activated' : 'User suspended', 'success');
    } catch {
      showToast('Failed to update user status', 'error');
    }
  }

  // ── Impersonate ─────────────────────────────────────────────────────────────

  async function loginAsUser(user: User) {
    impersonatingId = user.id;
    try {
      const result = await api.reseller.loginAsClient(user.id);
      const adminToken = localStorage.getItem('orbit_access_token');
      if (adminToken) localStorage.setItem('orbit_original_token', adminToken);
      api.setToken(result.access_token);
      localStorage.setItem('orbit_access_token', result.access_token);
      localStorage.setItem('orbit_impersonate_email', result.email);
      localStorage.setItem('orbit_impersonate_token', user.id);
      await goto('/dashboard', { invalidateAll: true });
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to impersonate user', 'error');
      impersonatingId = null;
    }
  }

  // ── Bulk actions ───────────────────────────────────────────────────────────

  function toggleSelectAll() {
    if (selectedIds.size === filteredUsers.length) {
      selectedIds = new Set();
    } else {
      selectedIds = new Set(filteredUsers.map(u => u.id));
    }
  }

  function toggleSelect(id: string) {
    if (selectedIds.has(id)) {
      selectedIds.delete(id);
    } else {
      selectedIds.add(id);
    }
    selectedIds = selectedIds;
  }

  async function bulkSuspend() {
    bulkLoading = true;
    showBulkMenu = false;
    let ok = 0;
    for (const id of selectedIds) {
      try { await api.reseller.suspendClient(id); ok++; } catch { /* skip */ }
    }
    users = users.map(u => selectedIds.has(u.id) ? { ...u, status: 'suspended' } as User : u);
    selectedIds = new Set();
    showToast(`${ok} user(s) suspended`, 'success');
    bulkLoading = false;
  }

  async function bulkDelete() {
    confirmBulkDelete = false;
    bulkLoading = true;
    showBulkMenu = false;
    let ok = 0;
    const ids = [...selectedIds];
    for (const id of ids) {
      try { await api.users.delete(id); ok++; } catch { /* skip */ }
    }
    users = users.filter(u => !ids.includes(u.id));
    selectedIds = new Set();
    showToast(`${ok} user(s) deleted`, 'success');
    bulkLoading = false;
  }

  // ── Helpers ─────────────────────────────────────────────────────────────────

  function showToast(msg: string, type: 'success' | 'error') {
    if (toastTimer) clearTimeout(toastTimer);
    toastMessage = msg;
    toastType = type;
    toastTimer = setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function avatarInitials(user: User): string {
    const name = user.display_name ?? user.email;
    const parts = name.split(/[\s@]/);
    return parts.length >= 2
      ? (parts[0][0] + parts[1][0]).toUpperCase()
      : name.slice(0, 2).toUpperCase();
  }

  function avatarColor(user: User): string {
    const colors = [
      'bg-blue-500/20 text-blue-400',
      'bg-purple-500/20 text-purple-400',
      'bg-green-500/20 text-green-400',
      'bg-amber-500/20 text-amber-400',
      'bg-red-500/20 text-red-400',
      'bg-indigo-500/20 text-indigo-400',
      'bg-pink-500/20 text-pink-400',
      'bg-cyan-500/20 text-cyan-400',
    ];
    const h = [...user.email].reduce((a, c) => a + c.charCodeAt(0), 0);
    return colors[h % colors.length];
  }

  function roleBadgeClass(role: User['role']): string {
    return {
      admin:    'bg-purple-500/10 text-purple-400 border border-purple-500/20',
      reseller: 'bg-amber-500/10 text-amber-400 border border-amber-500/20',
      user:     'bg-blue-500/10 text-blue-400 border border-blue-500/20',
    }[role] ?? 'bg-muted text-muted-foreground border border-border';
  }

  function formatDate(dateStr: string | null): string {
    if (!dateStr) return 'Never';
    const d = new Date(dateStr);
    const diff = Date.now() - d.getTime();
    if (diff < 60000)     return 'Just now';
    if (diff < 3600000)   return `${Math.floor(diff / 60000)}m ago`;
    if (diff < 86400000)  return `${Math.floor(diff / 3600000)}h ago`;
    if (diff < 604800000) return `${Math.floor(diff / 86400000)}d ago`;
    return d.toLocaleDateString();
  }

  function formatDateShort(dateStr: string | null): string {
    if (!dateStr) return '—';
    return new Date(dateStr).toLocaleDateString();
  }

  function eventColor(type: ActivityEvent['type']): string {
    const map: Record<string, string> = {
      login:            'bg-emerald-500/20 text-emerald-400',
      logout:           'bg-slate-500/20 text-slate-400',
      site_created:     'bg-blue-500/20 text-blue-400',
      site_deleted:     'bg-red-500/20 text-red-400',
      password_changed: 'bg-amber-500/20 text-amber-400',
      email_created:    'bg-cyan-500/20 text-cyan-400',
      backup_created:   'bg-indigo-500/20 text-indigo-400',
      db_created:       'bg-purple-500/20 text-purple-400',
    };
    return map[type] ?? 'bg-muted text-muted-foreground';
  }
</script>

<style>
  @keyframes fadeUp {
    from { opacity: 0; transform: translateY(6px); }
    to   { opacity: 1; transform: none; }
  }
  .fade-up { animation: fadeUp 0.25s ease-out both; }
</style>

<svelte:head>
  <title>{$t('users.title')} — JottiCP</title>
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
      <button on:click={() => void loadUsers()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- Header -->
  <div class="flex items-center justify-between gap-3 flex-wrap">
    <div>
      <h1 class="text-xl font-semibold text-foreground">{$t('users.title')}</h1>
      <p class="text-sm text-muted-foreground mt-0.5">
        {#if loading}{$t('users.loading')}{:else}{totalUsers} {$t('users.total_users')}{/if}
      </p>
    </div>
    <div class="flex items-center gap-2 flex-wrap">
      <!-- Activity log toggle -->
      <button
        on:click={toggleActivityLog}
        class="h-9 px-4 rounded-lg border border-border text-sm font-medium inline-flex items-center gap-2
               transition-all duration-150 active:scale-95
               {showActivityLog ? 'bg-muted text-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M9 5H7a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2V7a2 2 0 00-2-2h-2M9 5a2 2 0 002 2h2a2 2 0 002-2M9 5a2 2 0 012-2h2a2 2 0 012 2" />
        </svg>
        {showActivityLog ? 'Hide Activity' : 'Activity Log'}
      </button>
      <!-- Permissions button -->
      <button
        on:click={() => { showPermissionsModal = true; }}
        class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground
               hover:bg-muted hover:text-foreground inline-flex items-center gap-2
               transition-all duration-150 active:scale-95"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
        </svg>
        Permissions
      </button>
      <button
        on:click={openInviteModal}
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
               hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
        </svg>
        Invite User
      </button>
    </div>
  </div>

  <!-- Stats row -->
  {#if !loading}
    <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
      {#each [
        { label: 'Total Users',   value: totalUsers,     color: 'text-foreground'  },
        { label: 'Admins',        value: totalAdmins,    color: 'text-purple-400'  },
        { label: 'Resellers',     value: totalResellers, color: 'text-amber-400'   },
        { label: 'Regular Users', value: totalRegular,   color: 'text-blue-400'    },
      ] as stat}
        <div class="bg-card border border-border rounded-xl p-4
                    transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
          <p class="text-xs text-muted-foreground uppercase tracking-wider mb-1">{stat.label}</p>
          <p class="text-2xl font-bold {stat.color}">{stat.value}</p>
        </div>
      {/each}
    </div>
  {:else}
    <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
      {#each [1,2,3,4] as _}
        <div class="h-20 bg-muted rounded-xl animate-pulse"></div>
      {/each}
    </div>
  {/if}

  <!-- ── Activity Log ────────────────────────────────────────────────────── -->
  {#if showActivityLog}
    <div class="bg-card border border-border rounded-xl overflow-hidden fade-up">
      <div class="flex items-center justify-between px-4 py-3 border-b border-border">
        <h2 class="text-sm font-semibold text-foreground">Activity Log</h2>
        {#if activityLoading}
          <span class="text-xs text-muted-foreground">Loading...</span>
        {:else}
          <button on:click={loadActivityLog}
                  class="text-xs text-muted-foreground hover:text-foreground transition-colors">
            Refresh
          </button>
        {/if}
      </div>
      {#if activityLoading}
        <div class="space-y-1 p-3">
          {#each [1,2,3,4,5] as _}
            <div class="h-14 bg-muted rounded-lg animate-pulse"></div>
          {/each}
        </div>
      {:else if activityError}
        <p class="text-sm text-destructive text-center py-10">{activityError}</p>
      {:else if activityLog.length === 0}
        <p class="text-sm text-muted-foreground text-center py-10">No activity recorded yet.</p>
      {:else}
        <div class="space-y-0.5 p-2 max-h-96 overflow-y-auto">
          {#each activityLog as event (event.id)}
            <div class="flex items-start gap-3 p-3 rounded-lg hover:bg-muted/30 transition-colors">
              <!-- Icon -->
              <div class="w-8 h-8 rounded-full {eventColor(event.type)} flex items-center justify-center shrink-0 mt-0.5">
                {#if event.type === 'login'}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M11 16l-4-4m0 0l4-4m-4 4h14m-5 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h7a3 3 0 013 3v1" />
                  </svg>
                {:else if event.type === 'logout'}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M17 16l4-4m0 0l-4-4m4 4H7m6 4v1a3 3 0 01-3 3H6a3 3 0 01-3-3V7a3 3 0 013-3h4a3 3 0 013 3v1" />
                  </svg>
                {:else if event.type === 'site_created' || event.type === 'site_deleted'}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9" />
                  </svg>
                {:else if event.type === 'password_changed'}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                  </svg>
                {:else if event.type === 'email_created'}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z" />
                  </svg>
                {:else if event.type === 'backup_created'}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
                  </svg>
                {:else}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
                  </svg>
                {/if}
              </div>
              <!-- Content -->
              <div class="flex-1 min-w-0">
                <p class="text-sm text-foreground">
                  <span class="font-medium">{event.user_email}</span>
                  {' '}{event.description}
                </p>
                <p class="text-xs text-muted-foreground mt-0.5">{event.ip} · {formatDate(event.created_at)}</p>
              </div>
              <!-- Failed badge -->
              {#if event.success === false}
                <span class="text-xs text-red-400 bg-red-500/10 border border-red-500/20 px-1.5 py-0.5 rounded-full whitespace-nowrap shrink-0">
                  Failed
                </span>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <!-- Toolbar: search + filter + view toggle -->
  <div class="flex items-center gap-2 flex-wrap">
    <!-- Search -->
    <div class="relative flex-1 min-w-[200px] max-w-xs">
      <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none"
           fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
          d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
      </svg>
      <input
        type="search"
        bind:value={searchQuery}
        placeholder="Search users..."
        class="w-full h-9 pl-9 pr-3 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
      />
    </div>

    <!-- Role filter -->
    <select
      bind:value={roleFilter}
      class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground appearance-none focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
    >
      <option value="">All roles</option>
      <option value="admin">Admin</option>
      <option value="reseller">Reseller</option>
      <option value="user">User</option>
    </select>

    <!-- View toggle -->
    <div class="flex items-center rounded-lg border border-border overflow-hidden ml-auto">
      <button
        on:click={() => viewMode = 'table'}
        class="h-9 px-3 text-sm transition-colors
               {viewMode === 'table' ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground hover:bg-muted/50'}"
        title="Table view"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round"
            d="M3 10h18M3 14h18M10 3v18M4 3h16a1 1 0 011 1v16a1 1 0 01-1 1H4a1 1 0 01-1-1V4a1 1 0 011-1z" />
        </svg>
      </button>
      <button
        on:click={() => viewMode = 'cards'}
        class="h-9 px-3 text-sm border-l border-border transition-colors
               {viewMode === 'cards' ? 'bg-muted text-foreground' : 'text-muted-foreground hover:text-foreground hover:bg-muted/50'}"
        title="Card view"
      >
        <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
          <path stroke-linecap="round" stroke-linejoin="round"
            d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zm10 0a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zm10 0a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z" />
        </svg>
      </button>
    </div>
  </div>

  <!-- ── Users content ───────────────────────────────────────────────────── -->
  {#if loading}
    <div class="space-y-2">
      {#each [1,2,3,4,5] as _}
        <div class="h-16 bg-muted rounded-xl animate-pulse"></div>
      {/each}
    </div>

  {:else if filteredUsers.length === 0}
    <div class="bg-card border border-border rounded-xl p-16 text-center">
      <div class="w-12 h-12 rounded-full bg-muted flex items-center justify-center mx-auto mb-4">
        <svg class="w-6 h-6 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0z" />
        </svg>
      </div>
      <h3 class="text-sm font-medium text-foreground mb-1">
        {users.length === 0 ? 'No users yet' : 'No users match your search'}
      </h3>
      <p class="text-xs text-muted-foreground mb-4">
        {users.length === 0 ? 'Invite your first user to get started.' : 'Try adjusting your search or filter.'}
      </p>
      {#if users.length === 0}
        <button on:click={openInviteModal}
                class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                       hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95">
          Invite User
        </button>
      {/if}
    </div>

  {:else if viewMode === 'cards'}
    <!-- ── Cards view ────────────────────────────────────────────────── -->
    <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
      {#each filteredUsers as user (user.id)}
        {@const u = user as User & { status?: string; totp_enabled?: boolean; site_count?: number; email_count?: number; db_count?: number }}
        <div class="bg-card border border-border rounded-xl p-5
                    transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20
                    {selectedIds.has(user.id) ? 'ring-2 ring-primary/40' : ''}">
          <!-- Header row -->
          <div class="flex items-start justify-between mb-1">
            <div class="flex items-start gap-3 flex-1 min-w-0">
              <div class="w-10 h-10 rounded-full {avatarColor(user)} flex items-center justify-center font-medium text-sm shrink-0">
                {avatarInitials(user)}
              </div>
              <div class="flex-1 min-w-0">
                <p class="font-medium text-foreground truncate">{user.display_name || user.email.split('@')[0]}</p>
                <p class="text-xs text-muted-foreground truncate">{user.email}</p>
              </div>
            </div>
            <div class="flex items-center gap-2 shrink-0 ml-2">
              <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium {roleBadgeClass(user.role)}">
                {user.role}
              </span>
              <input
                type="checkbox"
                checked={selectedIds.has(user.id)}
                on:change={() => toggleSelect(user.id)}
                class="rounded border-border text-primary focus:ring-primary/50"
              />
            </div>
          </div>

          <!-- Stats row -->
          <div class="mt-3 pt-3 border-t border-border grid grid-cols-3 gap-2 text-center mb-3">
            <div>
              <p class="text-lg font-bold text-foreground">{u.site_count ?? 0}</p>
              <p class="text-[10px] text-muted-foreground">Sites</p>
            </div>
            <div>
              <p class="text-lg font-bold text-foreground">{u.email_count ?? 0}</p>
              <p class="text-[10px] text-muted-foreground">Emails</p>
            </div>
            <div>
              <p class="text-lg font-bold text-foreground">{u.db_count ?? 0}</p>
              <p class="text-[10px] text-muted-foreground">DBs</p>
            </div>
          </div>

          <!-- Meta + status -->
          <div class="flex items-center justify-between text-xs text-muted-foreground mb-3">
            <span>Joined {formatDateShort(user.created_at)}</span>
            {#if u.status === 'suspended'}
              <span class="flex items-center gap-1 text-amber-400">
                <span class="w-1.5 h-1.5 rounded-full bg-amber-400"></span>Suspended
              </span>
            {:else}
              <span class="flex items-center gap-1 text-emerald-400">
                <span class="w-1.5 h-1.5 rounded-full bg-emerald-400"></span>Active
              </span>
            {/if}
          </div>

          <!-- Actions -->
          <div class="flex items-center gap-1.5 pt-2 border-t border-border">
            <button on:click={() => openEditModal(user)}
                    class="flex-1 h-8 rounded-lg text-xs font-medium text-muted-foreground hover:bg-muted hover:text-foreground
                           transition-all duration-150 active:scale-95 border border-border">
              Edit
            </button>
            {#if user.id !== $currentUser?.id && user.role !== 'admin'}
              <button on:click={() => loginAsUser(user)}
                      disabled={impersonatingId === user.id}
                      class="h-8 px-3 rounded-lg text-xs font-medium text-primary bg-primary/10 hover:bg-primary/20
                             transition-all duration-150 active:scale-95 disabled:opacity-50">
                {impersonatingId === user.id ? '…' : 'Login as'}
              </button>
            {/if}
            {#if user.id !== $currentUser?.id}
              <button on:click={() => toggleSuspend(user)}
                      class="h-8 px-2.5 rounded-lg text-xs font-medium transition-all duration-150 active:scale-95
                             {u.status === 'suspended'
                               ? 'text-emerald-400 hover:bg-emerald-500/10'
                               : 'text-amber-400 hover:bg-amber-500/10'}">
                {u.status === 'suspended' ? 'Activate' : 'Suspend'}
              </button>
              <button on:click={() => openDeleteModal(user)}
                      class="h-8 w-8 rounded-lg flex items-center justify-center text-muted-foreground
                             hover:text-red-400 hover:bg-red-500/10 transition-all duration-150 active:scale-95">
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                </svg>
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>

  {:else}
    <!-- ── Table view ─────────────────────────────────────────────────── -->
    <div class="rounded-xl border border-border overflow-hidden">
      <table class="w-full text-sm">
        <thead class="bg-muted/50">
          <tr>
            <th class="px-4 py-3 w-8">
              <input
                type="checkbox"
                checked={selectedIds.size === filteredUsers.length && filteredUsers.length > 0}
                on:change={toggleSelectAll}
                class="rounded border-border text-primary focus:ring-primary/50"
              />
            </th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase text-left">User</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase text-left hidden sm:table-cell">Role</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase text-left hidden lg:table-cell">2FA</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase text-left hidden md:table-cell">Joined</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase text-left hidden lg:table-cell">Last Login</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase text-left hidden sm:table-cell">Status</th>
            <th class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase text-right sr-only">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each filteredUsers as user (user.id)}
            {@const u = user as User & { status?: string; totp_enabled?: boolean }}
            <tr class="border-t border-border transition-all duration-150 hover:bg-muted/40
                       {selectedIds.has(user.id) ? 'bg-primary/5' : ''}">
              <!-- Checkbox -->
              <td class="px-4 py-3">
                <input
                  type="checkbox"
                  checked={selectedIds.has(user.id)}
                  on:change={() => toggleSelect(user.id)}
                  class="rounded border-border text-primary focus:ring-primary/50"
                />
              </td>

              <!-- Avatar + Name + Email -->
              <td class="px-4 py-3">
                <div class="flex items-center gap-3">
                  <div class="w-8 h-8 rounded-full {avatarColor(user)} text-sm font-medium flex items-center justify-center shrink-0">
                    {avatarInitials(user)}
                  </div>
                  <div class="min-w-0">
                    <p class="font-medium text-foreground truncate">
                      {user.display_name ?? user.email}
                    </p>
                    {#if user.display_name}
                      <p class="text-xs text-muted-foreground truncate">{user.email}</p>
                    {/if}
                  </div>
                </div>
              </td>

              <!-- Role badge -->
              <td class="px-4 py-3 hidden sm:table-cell">
                <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium {roleBadgeClass(user.role)}">
                  {user.role}
                </span>
              </td>

              <!-- 2FA -->
              <td class="px-4 py-3 hidden lg:table-cell">
                {#if u.totp_enabled}
                  <svg class="w-5 h-5 text-emerald-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" title="2FA enabled">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                  </svg>
                {:else}
                  <svg class="w-5 h-5 text-muted-foreground/40" fill="none" viewBox="0 0 24 24" stroke="currentColor" title="2FA not enabled">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                      d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
                  </svg>
                {/if}
              </td>

              <!-- Joined -->
              <td class="px-4 py-3 text-xs text-muted-foreground hidden md:table-cell whitespace-nowrap">
                {formatDateShort(user.created_at)}
              </td>

              <!-- Last login -->
              <td class="px-4 py-3 text-xs text-muted-foreground hidden lg:table-cell whitespace-nowrap">
                {formatDate(user.last_login_at)}
              </td>

              <!-- Status -->
              <td class="px-4 py-3 hidden sm:table-cell">
                {#if u.status === 'suspended'}
                  <span class="inline-flex items-center gap-1.5 text-xs font-medium text-amber-400">
                    <span class="w-1.5 h-1.5 rounded-full bg-amber-400"></span>
                    Suspended
                  </span>
                {:else}
                  <span class="inline-flex items-center gap-1.5 text-xs font-medium text-emerald-400">
                    <span class="w-1.5 h-1.5 rounded-full bg-emerald-400"></span>
                    Active
                  </span>
                {/if}
              </td>

              <!-- Actions -->
              <td class="px-4 py-3">
                <div class="flex items-center gap-1 justify-end">
                  <!-- Edit -->
                  <button
                    on:click={() => openEditModal(user)}
                    class="h-8 px-3 rounded-lg text-xs font-medium text-muted-foreground
                           hover:bg-muted hover:text-foreground transition-all duration-150 active:scale-95"
                    title="Edit user"
                  >
                    Edit
                  </button>

                  <!-- Permissions -->
                  <button
                    on:click={() => showPermissionsModal = true}
                    class="h-8 px-3 rounded-lg text-xs font-medium text-muted-foreground
                           hover:bg-muted hover:text-foreground transition-all duration-150 active:scale-95"
                    title="Manage permissions"
                  >
                    Perms
                  </button>

                  <!-- Impersonate (not self, not admin) -->
                  {#if user.id !== $currentUser?.id && user.role !== 'admin'}
                    <button
                      on:click={() => loginAsUser(user)}
                      disabled={impersonatingId === user.id}
                      class="h-8 px-3 rounded-lg text-xs font-medium text-primary bg-primary/10
                             hover:bg-primary/20 transition-all duration-150 active:scale-95 disabled:opacity-50 whitespace-nowrap"
                      title="Impersonate user"
                    >
                      {impersonatingId === user.id ? '…' : 'Impersonate'}
                    </button>
                  {/if}

                  <!-- Suspend / Activate -->
                  {#if user.id !== $currentUser?.id}
                    <button
                      on:click={() => toggleSuspend(user)}
                      class="h-8 px-3 rounded-lg text-xs font-medium transition-all duration-150 active:scale-95
                             {(user as User & { status?: string }).status === 'suspended'
                               ? 'text-emerald-400 hover:bg-emerald-500/10'
                               : 'text-amber-400 hover:bg-amber-500/10'}"
                      title={(user as User & { status?: string }).status === 'suspended' ? 'Activate user' : 'Suspend user'}
                    >
                      {(user as User & { status?: string }).status === 'suspended' ? 'Activate' : 'Suspend'}
                    </button>

                    <!-- Delete -->
                    <button
                      on:click={() => openDeleteModal(user)}
                      class="h-8 w-8 rounded-lg flex items-center justify-center text-muted-foreground
                             hover:text-red-400 hover:bg-red-500/10 transition-all duration-150 active:scale-95"
                      title="Delete user"
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
  {/if}
</div>

<!-- ── Floating Bulk Action Bar ───────────────────────────────────────────── -->
{#if selectedIds.size > 0}
  <div class="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 bg-card border border-border rounded-full
              px-5 py-2.5 shadow-2xl flex items-center gap-4 text-sm fade-up">
    <span class="text-muted-foreground">{selectedIds.size} selected</span>
    <div class="h-4 w-px bg-border"></div>
    <button
      on:click={() => showToast('Bulk email is not yet supported', 'error')}
      class="text-foreground hover:text-primary transition-colors"
    >
      Send Email
    </button>
    <button
      on:click={bulkSuspend}
      disabled={bulkLoading}
      class="text-foreground hover:text-amber-400 transition-colors disabled:opacity-50"
    >
      Suspend
    </button>
    {#if confirmBulkDelete}
      <div class="flex items-center gap-1.5">
        <span class="text-xs text-destructive">Delete {selectedIds.size} users?</span>
        <button class="text-xs px-2 py-1 rounded bg-destructive text-white hover:bg-destructive/90" on:click={bulkDelete}>Yes</button>
        <button class="text-xs px-2 py-1 rounded bg-muted" on:click={() => confirmBulkDelete = false}>No</button>
      </div>
    {:else}
      <button
        on:click={() => confirmBulkDelete = true}
        disabled={bulkLoading}
        class="text-red-400 hover:text-red-300 transition-colors disabled:opacity-50"
      >
        Delete
      </button>
    {/if}
    <button
      on:click={() => selectedIds = new Set()}
      class="text-muted-foreground hover:text-foreground ml-2 transition-colors"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
        <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  </div>
{/if}

<!-- ── Permission Matrix Modal ────────────────────────────────────────────── -->
{#if showPermissionsModal}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showPermissionsModal = false}
       role="dialog" aria-modal="true" aria-label="Permission matrix">
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-4xl shadow-2xl max-h-[90vh] overflow-y-auto">
      <div class="flex items-center justify-between mb-5">
        <div>
          <h2 class="text-base font-semibold text-foreground">Permission Matrix</h2>
          <p class="text-xs text-muted-foreground mt-0.5">
            Click a cell to cycle permissions for non-admin users: Full → Own → None
          </p>
        </div>
        <button on:click={() => showPermissionsModal = false}
                class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <!-- Legend -->
      <div class="flex items-center gap-2 mb-4 flex-wrap">
        {#each [
          ['Full',   'bg-emerald-500/15 text-emerald-400 border border-emerald-500/25'],
          ['Own',    'bg-blue-500/15 text-blue-400 border border-blue-500/25'        ],
          ['None',   'bg-muted text-muted-foreground border border-border'            ],
          ['Custom', 'bg-amber-500/15 text-amber-400 border border-amber-500/25'     ],
        ] as [label, cls]}
          <span class="text-xs px-2 py-0.5 rounded-full {cls}">{label}</span>
        {/each}
        <span class="text-xs text-muted-foreground ml-2">Admins always have Full access (read-only)</span>
      </div>

      <!-- Matrix table -->
      <div class="overflow-x-auto">
        <table class="w-full text-sm border-collapse">
          <thead>
            <tr class="border-b border-border">
              <th class="text-left py-2 pr-4 text-xs font-medium text-muted-foreground min-w-[180px]">User</th>
              {#each permColumns as col}
                <th class="text-center py-2 px-2 text-xs font-medium text-muted-foreground whitespace-nowrap">
                  {col.label}
                </th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each users as user (user.id)}
              {@const perms = permOverrides[user.id] ?? defaultPerms(user)}
              <tr class="border-b border-border/50 hover:bg-muted/20 transition-colors">
                <td class="py-2.5 pr-4">
                  <div class="flex items-center gap-2">
                    <div class="w-7 h-7 rounded-full {avatarColor(user)} text-xs font-medium flex items-center justify-center shrink-0">
                      {avatarInitials(user)}
                    </div>
                    <div class="min-w-0">
                      <p class="text-xs text-foreground truncate max-w-[130px]">{user.email}</p>
                      <p class="text-[10px] text-muted-foreground">{user.role}</p>
                    </div>
                  </div>
                </td>
                {#each permColumns as col}
                  <td class="py-2.5 px-2 text-center">
                    <button
                      on:click={() => cyclePermLevel(user.id, col.key)}
                      disabled={user.role === 'admin'}
                      class="inline-flex items-center justify-center px-2 py-0.5 rounded-full text-xs font-medium
                             {permBadgeClass(perms[col.key])} transition-all duration-150
                             {user.role !== 'admin' ? 'hover:opacity-80 active:scale-95 cursor-pointer' : 'cursor-default opacity-70'}"
                      title={user.role === 'admin'
                        ? 'Admins always have full access'
                        : `Click to cycle ${col.label} permission`}
                    >
                      {perms[col.key]}
                    </button>
                  </td>
                {/each}
              </tr>
            {/each}
          </tbody>
        </table>
      </div>

      <div class="flex justify-end mt-5 gap-2">
        <button
          on:click={() => showPermissionsModal = false}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors"
        >
          Cancel
        </button>
        <button
          on:click={() => { showToast('Permission management is not yet supported by the API', 'error'); showPermissionsModal = false; }}
          class="h-9 px-5 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 transition-all duration-150 active:scale-95"
        >
          Save Permissions
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Invite User Modal ─────────────────────────────────────────────────── -->
{#if showInviteModal}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showInviteModal = false}
       role="dialog" aria-modal="true" aria-label="Invite user">
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">Invite User</h2>
        <button on:click={() => showInviteModal = false}
                class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <form on:submit={handleInvite} class="space-y-4">
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="invite-email">Email</label>
          <input id="invite-email" type="email" bind:value={inviteForm.email} required
                 class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="invite-name">Full Name</label>
          <input id="invite-name" type="text" bind:value={inviteForm.full_name}
                 placeholder="Optional"
                 class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        <div>
          <p class="block text-sm font-medium text-foreground mb-2">Role</p>
          <div class="flex gap-2">
            {#each [
              { value: 'user',     label: 'User'     },
              { value: 'reseller', label: 'Reseller' },
              { value: 'admin',    label: 'Admin'    },
            ] as opt}
              <label class="flex-1 cursor-pointer">
                <input type="radio" bind:group={inviteForm.role} value={opt.value} class="sr-only" />
                <div class="h-9 rounded-lg border text-sm font-medium flex items-center justify-center transition-all
                            {inviteForm.role === opt.value
                              ? 'border-primary bg-primary/10 text-primary'
                              : 'border-border text-muted-foreground hover:border-border/80 hover:bg-muted/50'}">
                  {opt.label}
                </div>
              </label>
            {/each}
          </div>
        </div>

        <div class="flex items-center justify-between p-3 bg-muted/30 rounded-lg border border-border">
          <div>
            <p class="text-sm font-medium text-foreground">Send invitation email</p>
            <p class="text-xs text-muted-foreground">User receives a link to set their password</p>
          </div>
          <button type="button"
                  on:click={() => { inviteForm.send_email = !inviteForm.send_email; if (!inviteForm.send_email) generatedPassword = generatePassword(); }}
                  class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors shrink-0
                         {inviteForm.send_email ? 'bg-primary' : 'bg-muted-foreground/30'}">
            <span class="inline-block h-3.5 w-3.5 transform rounded-full bg-background shadow transition-transform
                         {inviteForm.send_email ? 'translate-x-4' : 'translate-x-0.5'}"></span>
          </button>
        </div>

        {#if !inviteForm.send_email}
          <div>
            <label class="block text-sm font-medium text-foreground mb-1.5">Temporary Password</label>
            <div class="flex gap-2">
              <code class="flex-1 h-9 rounded-lg border border-border bg-background px-3 text-sm font-mono text-foreground flex items-center truncate">
                {generatedPassword}
              </code>
              <button type="button" on:click={copyPassword}
                      class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground transition-colors shrink-0 whitespace-nowrap">
                {passwordCopied ? 'Copied!' : 'Copy'}
              </button>
              <button type="button" on:click={() => generatedPassword = generatePassword()}
                      class="h-9 w-9 rounded-lg border border-border flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors shrink-0" title="Regenerate">
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                </svg>
              </button>
            </div>
          </div>
        {/if}

        {#if inviteError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5">
            {inviteError}
          </div>
        {/if}

        <div class="flex gap-2 pt-1">
          <button type="button" on:click={() => showInviteModal = false}
                  class="flex-1 h-9 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center gap-2 transition-colors">
            Cancel
          </button>
          <button type="submit" disabled={inviteLoading}
                  class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50">
            {inviteLoading ? 'Creating...' : 'Create User'}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<!-- ── Edit User Modal ────────────────────────────────────────────────────── -->
{#if showEditModal && editUser}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showEditModal = false}
       role="dialog" aria-modal="true" aria-label="Edit user">
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">Edit User</h2>
        <button on:click={() => showEditModal = false}
                class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground transition-colors">
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <form on:submit={handleEdit} class="space-y-4">
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="edit-name">Full Name</label>
          <input id="edit-name" type="text" bind:value={editForm.full_name}
                 class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="edit-email">Email</label>
          <input id="edit-email" type="email" bind:value={editForm.email} required
                 class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        <div>
          <p class="block text-sm font-medium text-foreground mb-2">Role</p>
          <div class="flex gap-2">
            {#each [
              { value: 'user',     label: 'User'     },
              { value: 'reseller', label: 'Reseller' },
              { value: 'admin',    label: 'Admin'    },
            ] as opt}
              <label class="flex-1 cursor-pointer">
                <input type="radio" bind:group={editForm.role} value={opt.value} class="sr-only" />
                <div class="h-9 rounded-lg border text-sm font-medium flex items-center justify-center transition-all
                            {editForm.role === opt.value
                              ? 'border-primary bg-primary/10 text-primary'
                              : 'border-border text-muted-foreground hover:bg-muted/50'}">
                  {opt.label}
                </div>
              </label>
            {/each}
          </div>
        </div>

        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="edit-password">
            New Password
            <span class="text-xs text-muted-foreground font-normal ml-1">(leave blank to keep current)</span>
          </label>
          <input id="edit-password" type="password" bind:value={editForm.new_password} minlength="8"
                 class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary" />
        </div>

        {#if editError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5">
            {editError}
          </div>
        {/if}

        <div class="flex gap-2 pt-1">
          <button type="button" on:click={() => showEditModal = false}
                  class="flex-1 h-9 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center gap-2 transition-colors">
            Cancel
          </button>
          <button type="submit" disabled={editLoading}
                  class="flex-1 h-9 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center justify-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50">
            {editLoading ? 'Saving...' : 'Save Changes'}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<!-- ── Delete Confirmation Modal ────────────────────────────────────────── -->
{#if showDeleteModal && deleteTarget}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => { showDeleteModal = false; deleteTarget = null; }}
       role="dialog" aria-modal="true" aria-label="Confirm delete">
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl">
      <div class="flex items-start gap-4 mb-5">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
          </svg>
        </div>
        <div>
          <h2 class="text-base font-semibold text-foreground">Delete User</h2>
          <p class="text-sm text-muted-foreground mt-1">
            This will permanently delete <strong class="text-foreground">{deleteTarget.email}</strong> and all their data.
            This action cannot be undone.
          </p>
        </div>
      </div>

      <div class="mb-4">
        <label class="block text-sm font-medium text-foreground mb-1.5">
          Type <code class="font-mono text-red-400">{deleteTarget.email}</code> to confirm
        </label>
        <input type="text" bind:value={deleteConfirmEmail}
               placeholder={deleteTarget.email}
               class="w-full h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-red-500/50 focus:border-red-500" />
      </div>

      <div class="flex gap-2">
        <button on:click={() => { showDeleteModal = false; deleteTarget = null; }}
                class="flex-1 h-9 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center justify-center gap-2 transition-colors">
          Cancel
        </button>
        <button
          on:click={handleDelete}
          disabled={deleteConfirmEmail !== deleteTarget.email || deleteLoading}
          class="flex-1 h-9 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center justify-center gap-2 transition-all duration-150 active:scale-95 disabled:opacity-50"
        >
          {deleteLoading ? 'Deleting...' : 'Delete User'}
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ─────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm shadow-lg
              flex items-center gap-2 fade-up"
       role="status" aria-live="polite">
    {#if toastType === 'success'}
      <svg class="w-4 h-4 text-emerald-400 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
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
