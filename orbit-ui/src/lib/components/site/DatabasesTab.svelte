<script lang="ts">
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import { api } from '$api/client';
  import type { Database, Site } from '$api/client';
  import { auth } from '$lib/stores/auth';

  // ── Props ──────────────────────────────────────────────────────────────────
  export let siteId: string;
  export let site: Site;

  // ── Local types ────────────────────────────────────────────────────────────
  interface DbUser {
    id: string;
    username: string;
    privileges: string[];
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

  // ── Constants ──────────────────────────────────────────────────────────────
  const dbColors: Record<string, string> = {
    mysql:      'bg-blue-500/10 text-blue-400 border border-blue-500/20',
    mariadb:    'bg-orange-500/10 text-orange-400 border border-orange-500/20',
    postgresql: 'bg-indigo-500/10 text-indigo-400 border border-indigo-500/20',
    ferretdb:   'bg-yellow-500/10 text-yellow-400 border border-yellow-500/20',
    surrealdb:  'bg-purple-500/10 text-purple-400 border border-purple-500/20',
  };

  const dbLabels: Record<string, string> = {
    mysql: 'MySQL', mariadb: 'MariaDB', postgresql: 'PostgreSQL',
    ferretdb: 'FerretDB', surrealdb: 'SurrealDB',
  };

  const ALL_PRIVILEGES = ['SELECT', 'INSERT', 'UPDATE', 'DELETE', 'ALL PRIVILEGES'];

  // ── State ──────────────────────────────────────────────────────────────────
  let databases: Database[] = [];
  let loading = true;

  // Expandable users panel
  let expandedDbId: string | null = null;
  let dbUsers: Record<string, DbUser[]> = {};
  let usersLoading: Record<string, boolean> = {};
  let usersError: Record<string, string> = {};
  let showAddUserForm: Record<string, boolean> = {};
  let newUser: { username: string; password: string; privileges: string[] } = {
    username: '', password: '', privileges: ['SELECT'],
  };

  // Modals
  let createModal = false;
  let connModal = false;
  let pwModal = false;
  let importModal = false;

  let selectedDb: Database | null = null;

  // Create DB form
  let newDb = { db_type: 'mysql' as Database['db_type'], db_name: '' };
  let createLoading = false;
  let createError = '';
  let newDbPassword = '';
  let showNewDbPassword = false;

  // Change password form
  let newPassword = '';
  let showPassword = false;
  let pwLoading = false;

  // Connection modal
  let showConnPassword = false;

  // Import modal
  let importFile: FileList | null = null;
  let importLoading = false;
  let importProgress = false;

  // Toast queue
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

  function formatBytes(b: number): string {
    if (b < 1024) return b + ' B';
    if (b < 1048576) return (b / 1024).toFixed(1) + ' KB';
    if (b < 1073741824) return (b / 1048576).toFixed(1) + ' MB';
    return (b / 1073741824).toFixed(1) + ' GB';
  }

  function showToast(message: string, type: 'success' | 'error' = 'success') {
    const id = ++toastCounter;
    toasts = [...toasts, { id, message, type }];
    setTimeout(() => { toasts = toasts.filter(t => t.id !== id); }, 4000);
  }

  function copyToClipboard(text: string) {
    navigator.clipboard.writeText(text).then(
      () => showToast('Copied to clipboard'),
      () => showToast('Copy failed', 'error'),
    );
  }

  function buildConnectionUri(db: Database): string {
    const proto = db.db_type === 'postgresql' ? 'postgresql' : 'mysql';
    // Password is never stored in the UI; show placeholder only
    return `${proto}://${db.db_user}:<password>@${db.db_host}:${db.db_port}/${db.db_name}`;
  }

  function closeOnEsc(e: KeyboardEvent) {
    if (e.key === 'Escape') closeAllModals();
  }

  function closeAllModals() {
    createModal = false;
    connModal = false;
    pwModal = false;
    importModal = false;
    selectedDb = null;
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────
  onMount(async () => {
    try {
      databases = await api.databases.list({ site_id: siteId });
    } catch {
      showToast('Failed to load databases', 'error');
    } finally {
      loading = false;
    }
  });

  // ── DB Users panel ─────────────────────────────────────────────────────────
  async function toggleUsers(db: Database) {
    if (expandedDbId === db.id) {
      expandedDbId = null;
      return;
    }
    expandedDbId = db.id;
    if (!dbUsers[db.id]) {
      await loadUsers(db.id);
    }
  }

  async function loadUsers(dbId: string) {
    usersLoading = { ...usersLoading, [dbId]: true };
    usersError = { ...usersError, [dbId]: '' };
    try {
      const res = await fetch(`/api/v1/databases/${dbId}/users`, { headers: authH() });
      if (res.status === 404 || res.status === 501) {
        usersError = { ...usersError, [dbId]: 'coming_soon' };
        return;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const users: DbUser[] = await res.json();
      dbUsers = { ...dbUsers, [dbId]: users };
    } catch (err: unknown) {
      const e = err as { message?: string };
      usersError = { ...usersError, [dbId]: e.message ?? 'Failed to load users' };
    } finally {
      usersLoading = { ...usersLoading, [dbId]: false };
    }
  }

  function openAddUserForm(dbId: string) {
    newUser = { username: '', password: generatePassword(), privileges: ['SELECT'] };
    showAddUserForm = { ...showAddUserForm, [dbId]: true };
  }

  function togglePrivilege(priv: string) {
    if (priv === 'ALL PRIVILEGES') {
      newUser.privileges = newUser.privileges.includes('ALL PRIVILEGES') ? [] : ['ALL PRIVILEGES'];
    } else {
      const without = newUser.privileges.filter(p => p !== 'ALL PRIVILEGES' && p !== priv);
      newUser.privileges = newUser.privileges.includes(priv) ? without : [...without, priv];
    }
  }

  async function addUser(dbId: string) {
    try {
      const res = await fetch(`/api/v1/databases/${dbId}/users`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify(newUser),
      });
      if (!res.ok) {
        const err = await res.json() as { message?: string };
        throw new Error(err.message ?? `HTTP ${res.status}`);
      }
      const created: DbUser = await res.json();
      dbUsers = { ...dbUsers, [dbId]: [...(dbUsers[dbId] ?? []), created] };
      showAddUserForm = { ...showAddUserForm, [dbId]: false };
      showToast('User created successfully');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to create user', 'error');
    }
  }

  async function deleteUser(dbId: string, uid: string) {
    try {
      const res = await fetch(`/api/v1/databases/${dbId}/users/${uid}`, {
        method: 'DELETE',
        headers: authH(),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      dbUsers = { ...dbUsers, [dbId]: (dbUsers[dbId] ?? []).filter(u => u.id !== uid) };
      showToast('User deleted');
    } catch {
      showToast('Failed to delete user', 'error');
    }
  }

  // ── Create DB ──────────────────────────────────────────────────────────────
  function openCreateModal() {
    newDb = { db_type: 'mysql', db_name: '' };
    newDbPassword = generatePassword();
    showNewDbPassword = false;
    createError = '';
    createModal = true;
  }

  async function handleCreate() {
    createError = '';
    if (!newDb.db_name.trim()) { createError = 'Database name is required'; return; }
    if (!/^[a-z0-9_]+$/i.test(newDb.db_name.trim())) {
      createError = 'Database name may only contain letters, numbers, and underscores';
      return;
    }
    createLoading = true;
    try {
      const db = await api.databases.create({
        db_type: newDb.db_type,
        db_name: newDb.db_name.trim(),
        site_id: siteId,
      });
      databases = [...databases, db];
      createModal = false;
      showToast('Database created successfully');
    } catch (err: unknown) {
      const e = err as { message?: string };
      createError = e.message ?? 'Failed to create database';
    } finally {
      createLoading = false;
    }
  }

  // ── Export ─────────────────────────────────────────────────────────────────
  function exportDb(db: Database) {
    window.location.href = '/api/v1/databases/' + db.id + '/export';
  }

  // ── Import ─────────────────────────────────────────────────────────────────
  function openImportModal(db: Database) {
    selectedDb = db;
    importFile = null;
    importLoading = false;
    importProgress = false;
    importModal = true;
  }

  async function handleImport() {
    if (!selectedDb || !importFile || importFile.length === 0) return;
    importLoading = true;
    importProgress = true;
    try {
      const form = new FormData();
      form.append('file', importFile[0]);
      const t = get(auth).token;
      const headers: Record<string, string> = t ? { 'Authorization': 'Bearer ' + t } : {};
      const res = await fetch(`/api/v1/databases/${selectedDb.id}/import`, {
        method: 'POST',
        headers,
        body: form,
      });
      if (!res.ok) {
        const err = await res.json() as { message?: string };
        throw new Error(err.message ?? `HTTP ${res.status}`);
      }
      importModal = false;
      showToast('Import completed successfully');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Import failed', 'error');
    } finally {
      importLoading = false;
      importProgress = false;
    }
  }

  // ── Connection string ──────────────────────────────────────────────────────
  function openConnModal(db: Database) {
    selectedDb = db;
    showConnPassword = false;
    connModal = true;
  }

  // ── PhpMyAdmin ─────────────────────────────────────────────────────────────
  async function openPhpMyAdmin(db: Database) {
    try {
      const { url } = await api.databases.getPhpMyAdminToken(db.id);
      window.open(url, '_blank');
    } catch {
      showToast('Failed to get phpMyAdmin token', 'error');
    }
  }

  // ── Change password ────────────────────────────────────────────────────────
  function openPwModal(db: Database) {
    selectedDb = db;
    newPassword = generatePassword();
    showPassword = false;
    pwLoading = false;
    pwModal = true;
  }

  async function handleChangePassword() {
    if (!selectedDb) return;
    pwLoading = true;
    try {
      await api.databases.changePassword(selectedDb.id, newPassword);
      pwModal = false;
      showToast('Password changed successfully');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to change password', 'error');
    } finally {
      pwLoading = false;
    }
  }

  // ── Delete ─────────────────────────────────────────────────────────────────
  let deleteTarget: Database | null = null;
  let deleteConfirmText = '';
  let deleting = false;

  function openDeleteModal(db: Database) {
    deleteTarget = db;
    deleteConfirmText = '';
  }
  function closeDeleteModal() {
    deleteTarget = null;
    deleteConfirmText = '';
    deleting = false;
  }
  async function confirmDelete() {
    if (!deleteTarget || deleteConfirmText !== deleteTarget.db_name) return;
    deleting = true;
    try {
      await api.databases.delete(deleteTarget.id);
      databases = databases.filter(d => d.id !== deleteTarget!.id);
      if (expandedDbId === deleteTarget.id) expandedDbId = null;
      showToast('Database deleted');
      closeDeleteModal();
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Failed to delete database', 'error');
      deleting = false;
    }
  }
  function deleteDatabase(db: Database) { openDeleteModal(db); }

  // ── Reactive ───────────────────────────────────────────────────────────────
  $: pwStrengthScore = pwStrength(newPassword);
  $: newDbPwStrength = pwStrength(newDbPassword);
  $: fullDbName = newDb.db_name ? `${site.unix_user}_${newDb.db_name}` : '';
</script>

<svelte:window on:keydown={closeOnEsc} />

<style>
  @keyframes fadeUp { from { opacity:0; transform:translateY(4px) } to { opacity:1; transform:none } }
  .tab-content { animation: fadeUp 0.2s ease-out both }
</style>

<!-- ── Main section ─────────────────────────────────────────────────────────── -->
<div class="tab-content space-y-4">

  <!-- Header -->
  <div class="flex items-center justify-between">
    <h2 class="text-lg font-semibold text-foreground">Databases</h2>
    <button
      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
      on:click={openCreateModal}
    >
      + New Database
    </button>
  </div>

  <!-- Skeleton -->
  {#if loading}
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <div class="p-4 space-y-3">
        {#each [1, 2, 3] as _}
          <div class="flex items-center gap-4">
            <div class="animate-pulse bg-muted rounded h-6 w-20"></div>
            <div class="animate-pulse bg-muted rounded h-5 w-40 flex-1"></div>
            <div class="animate-pulse bg-muted rounded h-5 w-24"></div>
            <div class="animate-pulse bg-muted rounded h-5 w-16"></div>
          </div>
        {/each}
      </div>
    </div>

  <!-- Empty state -->
  {:else if databases.length === 0}
    <div class="bg-card border border-border rounded-xl p-12 text-center">
      <svg class="mx-auto mb-4 w-12 h-12 text-muted-foreground/40" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
          d="M20 7C20 8.657 16.418 10 12 10C7.582 10 4 8.657 4 7M20 7C20 5.343 16.418 4 12 4C7.582 4 4 5.343 4 7M20 7V17C20 18.657 16.418 20 12 20C7.582 20 4 18.657 4 17V7M20 12C20 13.657 16.418 15 12 15C7.582 15 4 13.657 4 12" />
      </svg>
      <p class="text-muted-foreground font-medium mb-1">No databases yet</p>
      <p class="text-muted-foreground/60 text-sm mb-4">Create your first database to get started.</p>
      <button
        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors"
        on:click={openCreateModal}
      >
        Create your first database
      </button>
    </div>

  <!-- Table -->
  {:else}
    <div class="bg-card border border-border rounded-xl overflow-hidden">
      <table class="w-full text-sm">
        <thead>
          <tr class="bg-muted/30">
            <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Type</th>
            <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">DB Name</th>
            <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">DB User</th>
            <th class="text-left px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Size</th>
            <th class="text-right px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider">Actions</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-border">
          {#each databases as db (db.id)}
            <!-- Main row -->
            <tr class="hover:bg-muted/10 transition-colors">
              <td class="px-4 py-3">
                <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium {dbColors[db.db_type] ?? ''}">
                  {dbLabels[db.db_type] ?? db.db_type}
                </span>
              </td>
              <td class="px-4 py-3 font-mono text-foreground text-xs">{db.db_name}</td>
              <td class="px-4 py-3 text-muted-foreground text-xs font-mono">{db.db_user}</td>
              <td class="px-4 py-3 text-muted-foreground text-xs">{formatBytes(db.size_mb * 1048576)}</td>
              <td class="px-4 py-3">
                <div class="flex items-center justify-end gap-1">
                  <!-- Users -->
                  <button
                    title="DB Users"
                    class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors {expandedDbId === db.id ? 'bg-muted text-foreground' : ''}"
                    on:click={() => toggleUsers(db)}
                  >
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
                    </svg>
                  </button>

                  <!-- Export -->
                  <button
                    title="Export"
                    class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors"
                    on:click={() => exportDb(db)}
                  >
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                    </svg>
                  </button>

                  <!-- Import -->
                  <button
                    title="Import"
                    class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors"
                    on:click={() => openImportModal(db)}
                  >
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l4-4m0 0l4 4m-4-4v12" />
                    </svg>
                  </button>

                  <!-- Connection -->
                  <button
                    title="Connection String"
                    class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors"
                    on:click={() => openConnModal(db)}
                  >
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
                    </svg>
                  </button>

                  <!-- DB Manager (MySQL/MariaDB only) -->
                  {#if db.db_type === 'mysql' || db.db_type === 'mariadb'}
                  <button
                    title="Open DB Manager"
                    class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors"
                    on:click={() => openPhpMyAdmin(db)}
                  >
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                    </svg>
                  </button>
                  {/if}

                  <!-- Change Password -->
                  <button
                    title="Change Password"
                    class="h-8 w-8 flex items-center justify-center rounded-lg text-muted-foreground hover:bg-muted transition-colors"
                    on:click={() => openPwModal(db)}
                  >
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
                    </svg>
                  </button>

                  <!-- Delete -->
                  <button
                    title="Delete"
                    class="h-8 w-8 flex items-center justify-center rounded-lg text-red-400 hover:bg-red-500/10 transition-colors"
                    on:click={() => deleteDatabase(db)}
                  >
                    <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                    </svg>
                  </button>
                </div>
              </td>
            </tr>

            <!-- DB Users expandable panel -->
            {#if expandedDbId === db.id}
              <tr>
                <td colspan="5" class="px-4 pb-4 pt-0 bg-muted/5">
                  <div class="border border-border rounded-xl p-4 mt-1">
                    <div class="flex items-center justify-between mb-3">
                      <h3 class="text-sm font-semibold text-foreground">DB Users — {db.db_name}</h3>
                      {#if !showAddUserForm[db.id]}
                        <button
                          class="h-8 px-3 rounded-lg bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90 transition-colors"
                          on:click={() => openAddUserForm(db.id)}
                        >
                          + Add User
                        </button>
                      {/if}
                    </div>

                    {#if usersLoading[db.id]}
                      <div class="space-y-2">
                        {#each [1, 2] as _}
                          <div class="animate-pulse bg-muted rounded h-8"></div>
                        {/each}
                      </div>

                    {:else if usersError[db.id] === 'coming_soon'}
                      <p class="text-sm text-muted-foreground py-2">User management coming soon for this database type.</p>

                    {:else if usersError[db.id]}
                      <p class="text-sm text-red-400 py-2">{usersError[db.id]}</p>

                    {:else if (dbUsers[db.id] ?? []).length === 0 && !showAddUserForm[db.id]}
                      <p class="text-sm text-muted-foreground py-2">No users yet. Add a user to grant access.</p>

                    {:else}
                      <div class="space-y-2 mb-3">
                        {#each (dbUsers[db.id] ?? []) as user (user.id)}
                          <div class="flex items-center justify-between py-2 px-3 rounded-lg bg-background border border-border">
                            <div class="flex items-center gap-3">
                              <span class="font-mono text-xs text-foreground">{user.username}</span>
                              <div class="flex gap-1 flex-wrap">
                                {#each user.privileges as priv}
                                  <span class="inline-flex items-center px-1.5 py-0.5 rounded text-xs font-medium bg-muted text-muted-foreground">{priv}</span>
                                {/each}
                              </div>
                            </div>
                            <button
                              class="h-7 w-7 flex items-center justify-center rounded text-red-400 hover:bg-red-500/10 transition-colors flex-shrink-0"
                              title="Delete user"
                              on:click={() => deleteUser(db.id, user.id)}
                            >
                              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
                              </svg>
                            </button>
                          </div>
                        {/each}
                      </div>
                    {/if}

                    <!-- Add user form -->
                    {#if showAddUserForm[db.id]}
                      <div class="mt-3 pt-3 border-t border-border space-y-3">
                        <div class="grid grid-cols-2 gap-3">
                          <div>
                            <label class="block text-xs font-medium text-muted-foreground mb-1">Username</label>
                            <input
                              type="text"
                              bind:value={newUser.username}
                              placeholder="db_user"
                              class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
                            />
                          </div>
                          <div>
                            <label class="block text-xs font-medium text-muted-foreground mb-1">Password</label>
                            <div class="flex gap-1">
                              <input
                                type="text"
                                bind:value={newUser.password}
                                class="flex-1 h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
                              />
                              <button
                                class="h-9 w-9 flex items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex-shrink-0"
                                title="Generate"
                                on:click={() => { newUser.password = generatePassword(); }}
                              >
                                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                                    d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                                </svg>
                              </button>
                            </div>
                          </div>
                        </div>
                        <div>
                          <label class="block text-xs font-medium text-muted-foreground mb-1">Privileges</label>
                          <div class="flex flex-wrap gap-2">
                            {#each ALL_PRIVILEGES as priv}
                              <label class="flex items-center gap-1.5 cursor-pointer select-none">
                                <input
                                  type="checkbox"
                                  checked={newUser.privileges.includes(priv)}
                                  on:change={() => togglePrivilege(priv)}
                                  class="rounded border-border"
                                />
                                <span class="text-xs text-foreground">{priv}</span>
                              </label>
                            {/each}
                          </div>
                        </div>
                        <div class="flex gap-2">
                          <button
                            class="h-8 px-4 rounded-lg bg-primary text-primary-foreground text-xs font-medium hover:bg-primary/90 transition-colors"
                            on:click={() => addUser(db.id)}
                          >
                            Create User
                          </button>
                          <button
                            class="h-8 px-4 rounded-lg border border-border text-xs font-medium text-muted-foreground hover:bg-muted transition-colors"
                            on:click={() => { showAddUserForm = { ...showAddUserForm, [db.id]: false }; }}
                          >
                            Cancel
                          </button>
                        </div>
                      </div>
                    {/if}
                  </div>
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>


<!-- ── Create Database Modal ─────────────────────────────────────────────────── -->
{#if createModal}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeAllModals}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">New Database</h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={closeAllModals}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <!-- DB Type radio cards -->
      <div class="mb-4">
        <label class="block text-xs font-medium text-muted-foreground mb-2">Database Type</label>
        <div class="grid grid-cols-2 gap-2">
          {#each [
            { value: 'mysql',      label: 'MySQL',      cls: dbColors.mysql },
            { value: 'mariadb',    label: 'MariaDB',    cls: dbColors.mariadb },
            { value: 'postgresql', label: 'PostgreSQL', cls: dbColors.postgresql },
            { value: 'surrealdb',  label: 'SurrealDB',  cls: dbColors.surrealdb },
          ] as opt}
            <label
              class="flex items-center gap-2 p-3 rounded-xl border cursor-pointer transition-colors {newDb.db_type === opt.value ? 'border-primary bg-primary/5' : 'border-border hover:bg-muted/30'}"
            >
              <input type="radio" bind:group={newDb.db_type} value={opt.value} class="sr-only" />
              <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium {opt.cls}">{opt.label}</span>
              {#if newDb.db_type === opt.value}
                <svg class="w-4 h-4 text-primary ml-auto" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z" />
                </svg>
              {/if}
            </label>
          {/each}
        </div>
      </div>

      <!-- DB Name -->
      <div class="mb-4">
        <label class="block text-xs font-medium text-muted-foreground mb-1">Database Name</label>
        <input
          type="text"
          bind:value={newDb.db_name}
          placeholder="my_database"
          class="w-full h-10 px-3 rounded-lg border bg-background text-sm text-foreground focus:outline-none focus:ring-2
            {createError && (createError.includes('name') || createError.includes('only')) ? 'border-red-500 focus:ring-red-500/50' : 'border-border focus:ring-ring'}"
        />
        {#if createError && (createError.includes('name') || createError.includes('only'))}
          <p class="text-red-400 text-xs mt-1">{createError}</p>
        {:else if fullDbName}
          <p class="mt-1 text-xs text-muted-foreground">Will be created as: <span class="font-mono text-foreground">{fullDbName}</span></p>
        {/if}
      </div>

      <!-- Password -->
      <div class="mb-5">
        <label class="block text-xs font-medium text-muted-foreground mb-1">Initial Password</label>
        <div class="flex gap-2">
          <div class="relative flex-1">
            <input
              type={showNewDbPassword ? 'text' : 'password'}
              bind:value={newDbPassword}
              class="w-full h-10 px-3 pr-9 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
            />
            <button
              type="button"
              class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              on:click={() => { showNewDbPassword = !showNewDbPassword; }}
            >
              {#if showNewDbPassword}
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                </svg>
              {:else}
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                </svg>
              {/if}
            </button>
          </div>
          <button
            type="button"
            class="h-10 w-10 flex items-center justify-center rounded-lg border border-border text-muted-foreground hover:bg-muted transition-colors flex-shrink-0"
            title="Regenerate"
            on:click={() => { newDbPassword = generatePassword(); }}
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
          </button>
        </div>
        <!-- Strength meter -->
        <div class="flex gap-1 mt-2">
          {#each [1, 2, 3, 4] as seg}
            <div class="flex-1 h-1 rounded-full transition-colors {newDbPwStrength >= seg
              ? seg <= 1 ? 'bg-red-400' : seg <= 2 ? 'bg-orange-400' : seg <= 3 ? 'bg-yellow-400' : 'bg-green-400'
              : 'bg-muted'}"></div>
          {/each}
        </div>
      </div>

      {#if createError}
        <p class="text-sm text-red-400 mb-3">{createError}</p>
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
          disabled={createLoading || !newDb.db_name.trim()}
          on:click={handleCreate}
        >
          {createLoading ? 'Creating…' : 'Create Database'}
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Connection String Modal ───────────────────────────────────────────────── -->
{#if connModal && selectedDb}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeAllModals}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Connection Details</h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={closeAllModals}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="space-y-2 mb-4">
        {#each [
          { label: 'Host',     value: selectedDb.db_host },
          { label: 'Port',     value: String(selectedDb.db_port) },
          { label: 'Database', value: selectedDb.db_name },
          { label: 'User',     value: selectedDb.db_user },
        ] as row}
          <div class="flex items-center justify-between py-2 px-3 rounded-lg bg-muted/30">
            <div>
              <span class="text-xs text-muted-foreground w-20 inline-block">{row.label}</span>
              <span class="font-mono text-sm text-foreground">{row.value}</span>
            </div>
            <button
              class="h-7 px-2 text-xs rounded border border-border text-muted-foreground hover:bg-muted transition-colors"
              on:click={() => copyToClipboard(row.value)}
            >
              Copy
            </button>
          </div>
        {/each}
      </div>

      <!-- Connection URI -->
      <div class="rounded-lg bg-muted/30 border border-border p-3 mb-4">
        <div class="flex items-center justify-between mb-1">
          <span class="text-xs text-muted-foreground font-medium">Connection URI</span>
          <div class="flex gap-2">
            <span class="text-xs text-muted-foreground">Replace &lt;password&gt; with your DB password</span>
            <button
              class="h-6 px-2 text-xs rounded border border-border text-muted-foreground hover:bg-muted transition-colors"
              on:click={() => copyToClipboard(buildConnectionUri(selectedDb!))}
            >
              Copy
            </button>
          </div>
        </div>
        <code class="text-xs font-mono text-foreground break-all">{buildConnectionUri(selectedDb)}</code>
      </div>

      <div class="flex justify-end">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={closeAllModals}
        >
          Close
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Change Password Modal ─────────────────────────────────────────────────── -->
{#if pwModal && selectedDb}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeAllModals}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Change Password — <span class="font-mono">{selectedDb.db_name}</span></h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={closeAllModals}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="mb-4">
        <label class="block text-xs font-medium text-muted-foreground mb-1">New Password</label>
        <div class="flex gap-2">
          <div class="relative flex-1">
            <input
              type={showPassword ? 'text' : 'password'}
              bind:value={newPassword}
              class="w-full h-10 px-3 pr-9 rounded-lg border border-border bg-background text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring font-mono"
            />
            <button
              type="button"
              class="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
              on:click={() => { showPassword = !showPassword; }}
            >
              {#if showPassword}
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
            title="Generate"
            on:click={() => { newPassword = generatePassword(); }}
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
            </svg>
          </button>
        </div>
        <!-- Strength meter -->
        <div class="flex gap-1 mt-2">
          {#each [1, 2, 3, 4] as seg}
            <div class="flex-1 h-1 rounded-full transition-colors {pwStrengthScore >= seg
              ? seg <= 1 ? 'bg-red-400' : seg <= 2 ? 'bg-orange-400' : seg <= 3 ? 'bg-yellow-400' : 'bg-green-400'
              : 'bg-muted'}"></div>
          {/each}
        </div>
        <p class="text-xs text-muted-foreground mt-1">
          Strength: {['', 'Weak', 'Fair', 'Good', 'Strong'][pwStrengthScore]}
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
          disabled={pwLoading || newPassword.length < 8}
          on:click={handleChangePassword}
        >
          {pwLoading ? 'Saving…' : 'Change Password'}
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Import Modal ──────────────────────────────────────────────────────────── -->
{#if importModal && selectedDb}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    on:click|self={closeAllModals}
  >
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md mx-4 shadow-2xl">
      <div class="flex items-center justify-between mb-5">
        <h3 class="text-base font-semibold text-foreground">Import — <span class="font-mono">{selectedDb.db_name}</span></h3>
        <button class="text-muted-foreground hover:text-foreground transition-colors" on:click={closeAllModals}>
          <svg class="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <div class="mb-5">
        <label class="block text-xs font-medium text-muted-foreground mb-2">SQL / Dump File</label>
        <label class="flex flex-col items-center justify-center w-full h-28 border-2 border-dashed border-border rounded-xl cursor-pointer hover:bg-muted/20 transition-colors">
          <svg class="w-8 h-8 text-muted-foreground mb-2" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
              d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12" />
          </svg>
          {#if importFile && importFile.length > 0}
            <span class="text-sm font-medium text-foreground">{importFile[0].name}</span>
            <span class="text-xs text-muted-foreground">{formatBytes(importFile[0].size)}</span>
          {:else}
            <span class="text-sm text-muted-foreground">Click to select or drag file here</span>
            <span class="text-xs text-muted-foreground">.sql, .dump, .gz</span>
          {/if}
          <input
            type="file"
            accept=".sql,.dump,.gz"
            class="sr-only"
            bind:files={importFile}
          />
        </label>

        {#if importProgress}
          <div class="mt-3 flex items-center gap-2">
            <div class="flex-1 h-1.5 bg-muted rounded-full overflow-hidden">
              <div class="h-full bg-primary rounded-full animate-pulse" style="width: 60%"></div>
            </div>
            <span class="text-xs text-muted-foreground">Uploading…</span>
          </div>
        {/if}
      </div>

      <p class="text-xs text-muted-foreground mb-4">
        Warning: importing will execute all SQL statements in the file against <span class="font-mono text-foreground">{selectedDb.db_name}</span>.
      </p>

      <div class="flex justify-end gap-2">
        <button
          class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors"
          on:click={closeAllModals}
        >
          Cancel
        </button>
        <button
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 transition-colors disabled:opacity-50"
          disabled={importLoading || !importFile || importFile.length === 0}
          on:click={handleImport}
        >
          {importLoading ? 'Importing…' : 'Start Import'}
        </button>
      </div>
    </div>
  </div>
{/if}


<!-- ── Toast notifications ───────────────────────────────────────────────────── -->
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

<!-- ── Typed-confirmation delete modal ──────────────────────────────────────── -->
{#if deleteTarget}
  <div class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm" on:click|self={closeDeleteModal}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl">
      <div class="flex items-center gap-3 mb-3">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6M1 7h22M9 7V4a2 2 0 012-2h2a2 2 0 012 2v3"/>
          </svg>
        </div>
        <h3 class="text-base font-semibold text-foreground">Delete database</h3>
      </div>
      <p class="text-sm text-muted-foreground mb-4">
        This permanently deletes <strong class="text-foreground font-mono">{deleteTarget.db_name}</strong> and all its data.
        This action <strong class="text-red-400">cannot be undone</strong>.
      </p>
      <label class="block text-xs text-muted-foreground mb-1.5">
        Type <span class="font-mono text-foreground">{deleteTarget.db_name}</span> to confirm:
      </label>
      <input
        bind:value={deleteConfirmText}
        type="text"
        autocomplete="off"
        class="w-full h-9 px-3 rounded-lg border border-border bg-background text-sm text-foreground font-mono focus:outline-none focus:ring-2 focus:ring-red-500/50"
        placeholder={deleteTarget.db_name}
      />
      <div class="flex items-center gap-2 justify-end mt-5">
        <button on:click={closeDeleteModal} class="h-9 px-4 rounded-lg border border-border text-sm font-medium text-muted-foreground hover:bg-muted transition-colors">
          Cancel
        </button>
        <button
          on:click={confirmDelete}
          disabled={deleteConfirmText !== deleteTarget.db_name || deleting}
          class="h-9 px-4 rounded-lg bg-red-500 text-white text-sm font-semibold hover:bg-red-600 transition-colors disabled:opacity-40 disabled:cursor-not-allowed inline-flex items-center gap-2"
        >
          {#if deleting}
            <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24"><circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/><path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8v8H4z"/></svg>
            Deleting…
          {:else}
            Delete database
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}
