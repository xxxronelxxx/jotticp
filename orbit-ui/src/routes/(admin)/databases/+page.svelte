<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import type { Database } from '$api/client';
  import PageHeader from '$components/ui/PageHeader.svelte';
  import { databasesHelp } from '$lib/help/sites';
  import { debounce } from '$lib/utils';

  // ── Types ──────────────────────────────────────────────────────────────────

  interface SiteRef { id: string; domain: string }

  interface DbStats {
    size_bytes: number;
    table_count: number;
    connection_count: number;
  }

  // ── State ──────────────────────────────────────────────────────────────────
  let databases: Database[] = [];
  let sites: SiteRef[] = [];
  let loading = true;

  // Filters
  let searchQuery = '';   // committed (debounced) value used for filtering
  let searchInput = '';   // raw value bound to the input element
  let filterType = '';
  let filterTypePill = 'all'; // pill tab: all | mysql | postgresql | other
  let filterSite = '';

  // Debounced handler — commits searchQuery 250 ms after the user stops typing
  const onSearchInput = debounce((e: Event) => {
    searchQuery = (e.target as HTMLInputElement).value;
  }, 250);

  // Expanded stats rows
  let expandedRows: Set<string> = new Set();
  let statsCache: Record<string, DbStats | null> = {};
  let statsLoading: Set<string> = new Set();

  // Selected rows (bulk ops)
  let selectedIds: Set<string> = new Set();

  // Create modal
  let showCreateModal = false;
  let newDb = {
    db_type: 'mysql' as string,
    db_name: '',
    site_id: '',
    db_user: '',
    db_password: '',
  };
  let createLoading = false;
  let createError = '';
  let showPassword = false;
  let passwordStrength = 0;

  // Connect modal — with tabs
  let connectTarget: Database | null = null;
  let showConnPassword = false;
  let connTab: 'cli' | 'php' | 'python' | 'node' = 'cli';

  // Delete confirm modal
  let deleteTarget: Database | null = null;
  let deleteConfirmName = '';

  // Export/Import modal
  let exportTarget: Database | null = null;
  let importTarget: Database | null = null;
  let importFile: FileList | undefined;
  let importLoading = false;
  let importError = '';

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';

  // ── DB type config ─────────────────────────────────────────────────────────

  const DB_TYPES: Record<string, { label: string; badge: string; icon: string; color: string }> = {
    mysql:      { label: 'MySQL',      badge: 'bg-blue-500/10 text-blue-400 border border-blue-500/20',     icon: 'M',  color: 'text-blue-400'   },
    mariadb:    { label: 'MariaDB',    badge: 'bg-orange-500/10 text-orange-400 border border-orange-500/20', icon: 'Ma', color: 'text-orange-400' },
    postgresql: { label: 'PostgreSQL', badge: 'bg-indigo-500/10 text-indigo-400 border border-indigo-500/20', icon: 'P',  color: 'text-indigo-400' },
    postgres:   { label: 'PostgreSQL', badge: 'bg-indigo-500/10 text-indigo-400 border border-indigo-500/20', icon: 'P',  color: 'text-indigo-400' },
    ferretdb:   { label: 'FerretDB',  badge: 'bg-yellow-500/10 text-yellow-400 border border-yellow-500/20', icon: 'F',  color: 'text-yellow-400' },
    surrealdb:  { label: 'SurrealDB', badge: 'bg-purple-500/10 text-purple-400 border border-purple-500/20', icon: 'S',  color: 'text-purple-400' },
    valkey:     { label: 'Valkey',    badge: 'bg-red-500/10 text-red-400 border border-red-500/20',          icon: 'V',  color: 'text-red-400'    },
  };

  function dbCfg(type: string) {
    return DB_TYPES[type] ?? { label: type, badge: 'bg-muted text-muted-foreground border border-border', icon: 'D', color: 'text-muted-foreground' };
  }

  // Canonical types for create form (valkey/surrealdb omitted — need separate endpoints)
  const CREATE_TYPES = ['mysql', 'mariadb', 'postgresql'];

  // ── Status badge ───────────────────────────────────────────────────────────

  function statusBadge(status: string): { cls: string; label: string } {
    switch (status) {
      case 'active':        return { cls: 'bg-green-500/10 text-green-400 border border-green-500/20',  label: 'Active' };
      case 'provisioning':  return { cls: 'bg-amber-500/10 text-amber-400 border border-amber-500/20',  label: 'Provisioning' };
      case 'deleting':      return { cls: 'bg-red-500/10 text-red-400 border border-red-500/20',        label: 'Deleting' };
      case 'error':         return { cls: 'bg-red-500/10 text-red-400 border border-red-500/20',        label: 'Error' };
      default:              return { cls: 'bg-muted text-muted-foreground border border-border',        label: status };
    }
  }

  // ── Load ───────────────────────────────────────────────────────────────────

  onMount(async () => {
    if (typeof location !== "undefined" && new URLSearchParams(location.search).get("new") === "1") showCreateModal = true;
    loading = true;
    try {
      [databases, sites] = await Promise.all([
        api.databases.list(),
        api.sites.list().then((s: any[]) => s.map((x: any) => ({ id: x.id, domain: x.domain }))),
      ]);
      if (sites.length > 0) newDb.site_id = sites[0].id;
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load databases';
    } finally {
      loading = false;
    }
  });

  // ── Filtered list ──────────────────────────────────────────────────────────

  $: filteredDatabases = databases.filter(db => {
    const q = searchQuery.toLowerCase();
    const domain = siteDomain(db.site_id).toLowerCase();
    const matchSearch = !q
      || db.db_name.toLowerCase().includes(q)
      || db.db_type.toLowerCase().includes(q)
      || db.db_user.toLowerCase().includes(q)
      || domain.includes(q);
    // pill filter overrides dropdown when set
    let matchType = !filterType || db.db_type === filterType || (filterType === 'postgresql' && db.db_type === 'postgres');
    if (filterTypePill === 'mysql')      matchType = db.db_type === 'mysql' || db.db_type === 'mariadb';
    if (filterTypePill === 'postgresql') matchType = db.db_type === 'postgresql' || db.db_type === 'postgres';
    if (filterTypePill === 'other')      matchType = !['mysql','mariadb','postgresql','postgres'].includes(db.db_type);
    const matchSite = !filterSite || db.site_id === filterSite;
    return matchSearch && matchType && matchSite;
  });

  // ── Stats ──────────────────────────────────────────────────────────────────

  $: totalCount  = databases.length;
  $: mysqlCount  = databases.filter(db => db.db_type === 'mysql' || db.db_type === 'mariadb').length;
  $: pgCount     = databases.filter(db => db.db_type === 'postgresql' || db.db_type === 'postgres').length;
  $: otherCount  = databases.filter(db => !['mysql','mariadb','postgresql','postgres'].includes(db.db_type)).length;

  // ── Password strength ──────────────────────────────────────────────────────

  function measureStrength(pw: string): number {
    if (!pw) return 0;
    let score = 0;
    if (pw.length >= 12) score++;
    if (pw.length >= 16) score++;
    if (/[A-Z]/.test(pw)) score++;
    if (/[a-z]/.test(pw)) score++;
    if (/[0-9]/.test(pw)) score++;
    if (/[^a-zA-Z0-9]/.test(pw)) score++;
    return Math.min(score, 4);
  }

  $: passwordStrength = measureStrength(newDb.db_password);

  const strengthLabel = ['', 'Weak', 'Fair', 'Good', 'Strong'];
  const strengthColor = ['', 'bg-red-500', 'bg-amber-500', 'bg-blue-500', 'bg-green-500'];

  function generatePassword() {
    const chars = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789!@#$%^&*';
    let pw = '';
    for (let i = 0; i < 20; i++) {
      pw += chars[Math.floor(Math.random() * chars.length)];
    }
    newDb.db_password = pw;
    showPassword = true;
  }

  // Auto-generate username when db_name changes
  function refreshUsername() {
    if (newDb.db_name) {
      newDb.db_user = `orbit_${newDb.db_name.slice(0, 12)}`.toLowerCase().replace(/[^a-z0-9_]/g, '_');
    }
  }

  // ── Input validation ──────────────────────────────────────────────────────

  function validateDbName(v: string): string {
    if (!v.trim()) return 'Database name is required';
    if (!/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(v.trim())) return 'Database name must start with a letter or underscore, and contain only letters, numbers, and underscores';
    if (v.trim().length > 64) return 'Database name too long (max 64 characters)';
    return '';
  }

  function validateDbUsername(v: string): string {
    if (!v) return ''; // auto-generated, optional
    if (!/^[a-z_][a-z0-9_]*$/.test(v)) return 'Username may only contain lowercase letters, numbers and underscores';
    if (v.length > 32) return 'Username too long (max 32 characters)';
    return '';
  }

  // ── Create ─────────────────────────────────────────────────────────────────

  async function handleCreate(e: SubmitEvent) {
    e.preventDefault();
    createError = '';

    const nameErr = validateDbName(newDb.db_name);
    if (nameErr) { createError = nameErr; return; }

    if (newDb.db_user) {
      const userErr = validateDbUsername(newDb.db_user);
      if (userErr) { createError = userErr; return; }
    }

    if (!newDb.site_id) {
      createError = 'Please select a site';
      return;
    }

    createLoading = true;
    try {
      const db = await api.databases.create({
        db_type:  newDb.db_type,
        db_name:  newDb.db_name.trim(),
        site_id:  newDb.site_id,
      });
      databases = [db, ...databases];
      showCreateModal = false;
      resetCreateForm();
      showToast('Database creation queued', 'success');
    } catch (err: unknown) {
      createError = (err as Error).message ?? 'Failed to create database';
    } finally {
      createLoading = false;
    }
  }

  function resetCreateForm() {
    newDb = { db_type: 'mysql', db_name: '', site_id: sites[0]?.id ?? '', db_user: '', db_password: '' };
    createError = '';
    showPassword = false;
  }

  // ── Delete ─────────────────────────────────────────────────────────────────

  async function confirmDelete() {
    if (!deleteTarget) return;
    if (deleteConfirmName !== deleteTarget.db_name) {
      showToast('Database name does not match', 'error');
      return;
    }
    const target = deleteTarget;
    deleteTarget = null;
    deleteConfirmName = '';
    try {
      await api.databases.delete(target.id);
      databases = databases.filter(d => d.id !== target.id);
      showToast('Database deleted', 'success');
    } catch (err: unknown) {
      showToast((err as Error).message ?? 'Failed to delete database', 'error');
    }
  }

  // ── Expand / Stats row ─────────────────────────────────────────────────────

  async function toggleExpand(db: Database) {
    if (expandedRows.has(db.id)) {
      expandedRows.delete(db.id);
      expandedRows = new Set(expandedRows);
      return;
    }
    expandedRows.add(db.id);
    expandedRows = new Set(expandedRows);

    if (statsCache[db.id] !== undefined) return;
    statsLoading.add(db.id);
    statsLoading = new Set(statsLoading);
    try {
      const stats = await api.databases.stats(db.id);
      statsCache[db.id] = stats as DbStats;
    } catch {
      statsCache[db.id] = null; // null = placeholder
    } finally {
      statsLoading.delete(db.id);
      statsLoading = new Set(statsLoading);
    }
  }

  // ── Bulk operations ────────────────────────────────────────────────────────

  function toggleSelect(id: string) {
    if (selectedIds.has(id)) selectedIds.delete(id);
    else selectedIds.add(id);
    selectedIds = new Set(selectedIds);
  }

  function clearSelection() {
    selectedIds = new Set();
  }

  async function bulkDelete() {
    if (!selectedIds.size) return;
    const ids = [...selectedIds];
    clearSelection();
    for (const id of ids) {
      try {
        await api.databases.delete(id);
        databases = databases.filter(d => d.id !== id);
      } catch { /* continue */ }
    }
    showToast(`Deleted ${ids.length} database(s)`, 'success');
  }

  // ── phpMyAdmin 1-click (MySQL/MariaDB only) ────────────────────────────────
  async function openPhpMyAdmin(db: Database) {
    try {
      const { url } = await api.databases.getPhpMyAdminToken(db.id);
      window.open(url, '_blank');
    } catch {
      showToast('Failed to open phpMyAdmin', 'error');
    }
  }

  // ── Connect (connection string modal) ─────────────────────────────────────

  function getConnectionString(db: Database): string {
    const host = db.db_host ?? 'localhost';
    const port = db.db_port ?? defaultPort(db.db_type);
    return `${normalizeType(db.db_type)}://${db.db_user}@${host}:${port}/${db.db_name}`;
  }

  function getSnippet(db: Database, tab: 'cli' | 'php' | 'python' | 'node'): string {
    const host = db.db_host ?? 'localhost';
    const port = db.db_port ?? defaultPort(db.db_type);
    const user = db.db_user;
    const name = db.db_name;
    const pass = 'YOUR_PASSWORD';
    const isMySQL = db.db_type === 'mysql' || db.db_type === 'mariadb';
    const isPg    = db.db_type === 'postgresql' || db.db_type === 'postgres';

    switch (tab) {
      case 'cli':
        if (isMySQL)  return `mysql -h ${host} -P ${port} -u ${user} -p ${name}`;
        if (isPg)     return `psql -h ${host} -p ${port} -U ${user} -d ${name}`;
        return `${normalizeType(db.db_type)}://${user}@${host}:${port}/${name}`;

      case 'php':
        if (isMySQL)  return `new PDO('mysql:host=${host};port=${port};dbname=${name}', '${user}', '${pass}');`;
        if (isPg)     return `new PDO('pgsql:host=${host};port=${port};dbname=${name}', '${user}', '${pass}');`;
        return `/* PDO DSN for ${db.db_type} — see driver docs */`;

      case 'python':
        if (isMySQL)  return `import mysql.connector\ncnx = mysql.connector.connect(\n    host='${host}',\n    port=${port},\n    database='${name}',\n    user='${user}',\n    password='${pass}'\n)`;
        if (isPg)     return `import psycopg2\ncnx = psycopg2.connect(\n    host='${host}',\n    port=${port},\n    dbname='${name}',\n    user='${user}',\n    password='${pass}'\n)`;
        return `# See driver docs for ${db.db_type}`;

      case 'node':
        if (isMySQL)  return `import mysql2 from 'mysql2/promise';\nconst conn = await mysql2.createConnection({\n  host: '${host}',\n  port: ${port},\n  database: '${name}',\n  user: '${user}',\n  password: '${pass}'\n});`;
        if (isPg)     return `import { Client } from 'pg';\nconst client = new Client({\n  host: '${host}',\n  port: ${port},\n  database: '${name}',\n  user: '${user}',\n  password: '${pass}'\n});\nawait client.connect();`;
        return `// See driver docs for ${db.db_type}`;
    }
  }

  function normalizeType(t: string): string {
    if (t === 'postgresql' || t === 'postgres') return 'postgresql';
    return t;
  }

  function defaultPort(type: string): number {
    switch (type) {
      case 'mysql':
      case 'mariadb':     return 3306;
      case 'postgresql':
      case 'postgres':    return 5432;
      case 'ferretdb':    return 27017;
      case 'surrealdb':   return 8000;
      case 'valkey':      return 6379;
      default:            return 3306;
    }
  }

  async function copyText(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast('Copied to clipboard', 'success');
    } catch {
      showToast('Failed to copy', 'error');
    }
  }

  // ── Export / Import ─────────────────────────────────────────────────────────

  async function handleExport(db: Database) {
    exportTarget = db;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch(`/api/v1/databases/${db.id}/export`, {
        headers: {
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (res.status === 501) return;
      if (res.ok) {
        showToast('Export started', 'success');
        exportTarget = null;
      }
    } catch { /* Still show the modal */ }
  }

  async function handleImport(e: SubmitEvent) {
    e.preventDefault();
    if (!importTarget || !importFile?.length) return;
    importError = '';
    importLoading = true;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      const res = await fetch(`/api/v1/databases/${importTarget.id}/import`, {
        method: 'POST',
        headers: {
          'Accept': 'application/json',
          ...(token ? { 'Authorization': `Bearer ${token}` } : {}),
        },
      });
      if (res.status === 501) {
        const body = await res.json();
        importError = body.message ?? 'Import not available yet. Use phpMyAdmin.';
        return;
      }
      if (res.ok) {
        showToast('Import queued', 'success');
        importTarget = null;
      }
    } catch {
      importError = 'Import is not available yet. Please use phpMyAdmin.';
    } finally {
      importLoading = false;
    }
  }

  // ── Helper: site domain lookup ─────────────────────────────────────────────

  function siteDomain(siteId: string | null): string {
    if (!siteId) return '—';
    return sites.find(s => s.id === siteId)?.domain ?? siteId.slice(0, 8) + '…';
  }

  function formatSize(mb: number): string {
    if (!mb) return '—';
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
    return `${mb} MB`;
  }

  function formatBytes(bytes: number): string {
    if (!bytes) return '—';
    if (bytes >= 1073741824) return `${(bytes / 1073741824).toFixed(1)} GB`;
    if (bytes >= 1048576) return `${(bytes / 1048576).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${bytes} B`;
  }

  function showToast(msg: string, type: 'success' | 'error') {
    toastMessage = msg;
    toastType = type;
    setTimeout(() => { toastMessage = ''; }, 4000);
  }
</script>

<svelte:head>
  <title>Databases — OrbitCP</title>
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
      <button on:click={() => window.location.reload()}
              class="ml-auto text-xs font-semibold underline underline-offset-2 hover:no-underline">
        Retry
      </button>
    </div>
  {/if}

  <!-- Header -->
  <PageHeader
    title="Databases"
    subtitle="Global database management across all sites"
    helpTitle={databasesHelp.title}
    helpContent={databasesHelp.content}
  >
    <button
      on:click={() => { showCreateModal = true; resetCreateForm(); }}
      class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
      </svg>
      Create Database
      {#if !loading}
        <span class="text-primary-foreground/70 text-xs">({databases.length})</span>
      {/if}
    </button>
  </PageHeader>

  <!-- Stats row -->
  {#if !loading}
    <div class="grid grid-cols-2 sm:grid-cols-4 gap-4 fade-up">
      <!-- Total -->
      <div class="bg-card border border-border rounded-xl p-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <div class="text-2xl font-bold text-foreground">{totalCount}</div>
        <div class="text-xs text-muted-foreground mt-0.5">Total databases</div>
      </div>

      <!-- MySQL/MariaDB -->
      <div class="bg-card border border-border rounded-xl p-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <div class="flex items-center gap-2 mb-1">
          <div class="text-2xl font-bold text-foreground">{mysqlCount}</div>
          <span class="px-1.5 py-0.5 rounded-full text-xs font-medium bg-blue-500/10 text-blue-400 border border-blue-500/20">MySQL</span>
        </div>
        <div class="text-xs text-muted-foreground">MySQL / MariaDB</div>
      </div>

      <!-- PostgreSQL -->
      <div class="bg-card border border-border rounded-xl p-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <div class="flex items-center gap-2 mb-1">
          <div class="text-2xl font-bold text-foreground">{pgCount}</div>
          <span class="px-1.5 py-0.5 rounded-full text-xs font-medium bg-indigo-500/10 text-indigo-400 border border-indigo-500/20">PG</span>
        </div>
        <div class="text-xs text-muted-foreground">PostgreSQL</div>
      </div>

      <!-- Other -->
      <div class="bg-card border border-border rounded-xl p-4 transition-all duration-200 hover:-translate-y-0.5 hover:shadow-xl hover:shadow-black/20">
        <div class="text-2xl font-bold text-foreground">{otherCount}</div>
        <div class="text-xs text-muted-foreground mt-0.5">FerretDB / SurrealDB / Valkey</div>
      </div>
    </div>
  {/if}

  <!-- Filter bar -->
  <div class="space-y-3">
    <!-- Search + dropdowns row -->
    <div class="flex flex-wrap gap-3 items-center">
      <!-- Search -->
      <div class="relative flex-1 min-w-48 max-w-xs">
        <svg class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground pointer-events-none"
             fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
        </svg>
        <input
          type="search"
          value={searchInput}
          on:input={(e) => { searchInput = (e.target as HTMLInputElement).value; onSearchInput(e); }}
          placeholder="Search name, type, user, domain…"
          class="h-9 w-full pl-9 pr-3 rounded-lg border border-border bg-background text-sm text-foreground placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
        />
      </div>

      <!-- Site filter -->
      {#if sites.length > 0}
        <select
          bind:value={filterSite}
          class="h-9 rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
        >
          <option value="">All sites</option>
          {#each sites as site}
            <option value={site.id}>{site.domain}</option>
          {/each}
        </select>
      {/if}

      {#if searchQuery || filterSite || filterTypePill !== 'all'}
        <button
          on:click={() => { searchQuery = ''; searchInput = ''; filterType = ''; filterTypePill = 'all'; filterSite = ''; }}
          class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5"
        >
          <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
          Clear
        </button>
      {/if}
    </div>

    <!-- Type pill tabs -->
    <div class="flex items-center gap-1.5 flex-wrap">
      {#each [
        { key: 'all',        label: 'All',        count: totalCount },
        { key: 'mysql',      label: 'MySQL',      count: mysqlCount },
        { key: 'postgresql', label: 'PostgreSQL', count: pgCount },
        { key: 'other',      label: 'Other',      count: otherCount },
      ] as pill}
        <button
          on:click={() => filterTypePill = pill.key}
          class="h-7 px-3 rounded-full text-xs font-medium transition-all duration-150
            {filterTypePill === pill.key
              ? 'bg-primary text-primary-foreground'
              : 'bg-muted/40 text-muted-foreground hover:bg-muted hover:text-foreground'}"
        >
          {pill.label}
          <span class="ml-1 opacity-70">({pill.count})</span>
        </button>
      {/each}
    </div>
  </div>

  <!-- Table -->
  {#if loading}
    <div class="space-y-2">
      {#each [1, 2, 3, 4, 5] as _}
        <div class="h-14 bg-muted rounded-xl animate-pulse"></div>
      {/each}
    </div>
  {:else if filteredDatabases.length === 0}
    <div class="bg-card border border-border rounded-xl p-16 flex flex-col items-center gap-4 text-center fade-up">
      <div class="w-14 h-14 rounded-full bg-muted flex items-center justify-center">
        <svg class="w-7 h-7 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
            d="M4 7v10c0 2.21 3.582 4 8 4s8-1.79 8-4V7M4 7c0 2.21 3.582 4 8 4s8-1.79 8-4M4 7c0-2.21 3.582-4 8-4s8 1.79 8 4" />
        </svg>
      </div>
      <div>
        <p class="text-foreground font-medium">
          {databases.length === 0 ? 'No databases yet' : 'No databases match your filters'}
        </p>
        <p class="text-muted-foreground text-sm mt-1">
          {databases.length === 0 ? 'Create a database to get started.' : 'Try adjusting your search or filters.'}
        </p>
      </div>
      {#if databases.length === 0}
        <button
          on:click={() => { showCreateModal = true; resetCreateForm(); }}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          Create Database
        </button>
      {/if}
    </div>
  {:else}
    <div class="rounded-xl border border-border overflow-x-auto fade-up">
      <table class="w-full">
        <thead class="bg-muted/50">
          <tr>
            <th class="px-4 py-3 text-left w-8">
              <!-- bulk select header placeholder -->
            </th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase">Database</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden sm:table-cell">Type</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden md:table-cell">Associated Site</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden lg:table-cell">Username</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden lg:table-cell">Size</th>
            <th class="px-4 py-3 text-left text-xs text-muted-foreground uppercase hidden sm:table-cell">Status</th>
            <th class="px-4 py-3 text-right text-xs text-muted-foreground uppercase">Actions</th>
          </tr>
        </thead>
        <tbody>
          {#each filteredDatabases as db (db.id)}
            {@const cfg = dbCfg(db.db_type)}
            {@const sbadge = statusBadge(db.status ?? 'active')}
            {@const isExpanded = expandedRows.has(db.id)}
            {@const isSelected = selectedIds.has(db.id)}

            <!-- Main row -->
            <tr class="border-t border-border hover:bg-muted/30 transition-colors {isSelected ? 'bg-primary/5' : ''}">
              <td class="px-4 py-3">
                <input
                  type="checkbox"
                  checked={isSelected}
                  on:change={() => toggleSelect(db.id)}
                  class="w-3.5 h-3.5 rounded border-border text-primary focus:ring-primary/50 cursor-pointer"
                />
              </td>
              <td class="px-4 py-3 text-sm">
                <div class="flex items-center gap-2">
                  <!-- DB type icon badge (cylinder-style with letter overlay) -->
                  <span class="inline-flex items-center justify-center w-7 h-7 rounded-md {cfg.badge} text-xs font-bold font-mono shrink-0">
                    {cfg.icon}
                  </span>
                  <span class="font-mono font-medium text-foreground">{db.db_name}</span>
                </div>
              </td>
              <td class="px-4 py-3 hidden sm:table-cell">
                <span class="px-2 py-0.5 rounded-full text-xs font-medium {cfg.badge}">{cfg.label}</span>
              </td>
              <td class="px-4 py-3 hidden md:table-cell text-sm">
                {#if db.site_id}
                  <a href="/websites/{db.site_id}"
                     class="text-primary hover:underline text-sm">{siteDomain(db.site_id)}</a>
                {:else}
                  <span class="text-muted-foreground">—</span>
                {/if}
              </td>
              <td class="px-4 py-3 hidden lg:table-cell">
                <span class="font-mono text-xs text-muted-foreground">{db.db_user}</span>
              </td>
              <td class="px-4 py-3 hidden lg:table-cell">
                <span class="text-xs text-muted-foreground">{formatSize(db.size_mb)}</span>
              </td>
              <td class="px-4 py-3 hidden sm:table-cell">
                <span class="px-2 py-0.5 rounded-full text-xs font-medium {sbadge.cls}">{sbadge.label}</span>
              </td>
              <td class="px-4 py-3">
                <div class="flex items-center gap-1 justify-end">
                  <!-- Stats expand -->
                  <button
                    on:click={() => toggleExpand(db)}
                    class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                    title="Show database stats"
                  >
                    <svg class="w-3.5 h-3.5 transition-transform duration-200 {isExpanded ? 'rotate-180' : ''}"
                         fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
                    </svg>
                    <span class="hidden sm:inline">Stats</span>
                  </button>

                  <!-- phpMyAdmin (MySQL/MariaDB only) -->
                  {#if db.db_type === 'mysql' || db.db_type === 'mariadb'}
                  <button
                    on:click={() => openPhpMyAdmin(db)}
                    class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                    title="Open in phpMyAdmin"
                  >
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                    </svg>
                    <span class="hidden md:inline">phpMyAdmin</span>
                  </button>
                  {/if}

                  <!-- Connect -->
                  <button
                    on:click={() => { connectTarget = db; showConnPassword = false; connTab = 'cli'; }}
                    class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                    title="Connection string"
                  >
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
                    </svg>
                    <span class="hidden sm:inline">Connect</span>
                  </button>

                  <!-- Export -->
                  <button
                    on:click={() => handleExport(db)}
                    class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                    title="Export database"
                  >
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                    </svg>
                    <span class="hidden md:inline">Export</span>
                  </button>

                  <!-- Import -->
                  <button
                    on:click={() => { importTarget = db; importError = ''; importFile = undefined; }}
                    class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                    title="Import database"
                  >
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l4-4m0 0l4 4m-4-4v12" />
                    </svg>
                    <span class="hidden md:inline">Import</span>
                  </button>

                  <!-- Delete -->
                  <button
                    on:click={() => { deleteTarget = db; deleteConfirmName = ''; }}
                    class="h-9 px-3 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
                    title="Delete database"
                  >
                    <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                        d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                    </svg>
                    <span class="hidden md:inline">Delete</span>
                  </button>
                </div>
              </td>
            </tr>

            <!-- Expandable stats row -->
            {#if isExpanded}
              <tr class="border-t border-border bg-muted/10">
                <td colspan="8" class="px-6 py-4 fade-up">
                  {#if statsLoading.has(db.id)}
                    <div class="flex items-center gap-3 text-sm text-muted-foreground">
                      <svg class="w-4 h-4 animate-spin text-primary" fill="none" viewBox="0 0 24 24">
                        <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"/>
                        <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"/>
                      </svg>
                      Loading stats…
                    </div>
                  {:else}
                    {@const stats = statsCache[db.id]}
                    <div class="grid grid-cols-2 sm:grid-cols-4 gap-4">
                      <!-- Tables -->
                      <div class="flex items-center gap-2.5">
                        <div class="p-2 rounded-lg bg-blue-500/10">
                          <svg class="w-4 h-4 text-blue-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 10h18M3 14h18M10 4v16M14 4v16M3 6a1 1 0 011-1h16a1 1 0 011 1v12a1 1 0 01-1 1H4a1 1 0 01-1-1V6z"/>
                          </svg>
                        </div>
                        <div>
                          <div class="text-sm font-semibold text-foreground">{stats?.table_count ?? '—'}</div>
                          <div class="text-xs text-muted-foreground">Tables</div>
                        </div>
                      </div>
                      <!-- Size -->
                      <div class="flex items-center gap-2.5">
                        <div class="p-2 rounded-lg bg-green-500/10">
                          <svg class="w-4 h-4 text-green-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 8h14M5 8a2 2 0 110-4h14a2 2 0 110 4M5 8v10a2 2 0 002 2h10a2 2 0 002-2V8m-9 4h4"/>
                          </svg>
                        </div>
                        <div>
                          <div class="text-sm font-semibold text-foreground">{stats ? formatBytes(stats.size_bytes) : '—'}</div>
                          <div class="text-xs text-muted-foreground">Total size</div>
                        </div>
                      </div>
                      <!-- Connections -->
                      <div class="flex items-center gap-2.5">
                        <div class="p-2 rounded-lg bg-purple-500/10">
                          <svg class="w-4 h-4 text-purple-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"/>
                          </svg>
                        </div>
                        <div>
                          <div class="text-sm font-semibold text-foreground">{stats?.connection_count ?? '—'}</div>
                          <div class="text-xs text-muted-foreground">Active connections</div>
                        </div>
                      </div>
                      <!-- Placeholder info when 501 -->
                      {#if stats === null}
                        <div class="col-span-2 sm:col-span-1 flex items-center gap-2.5">
                          <div class="p-2 rounded-lg bg-amber-500/10">
                            <svg class="w-4 h-4 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"/>
                            </svg>
                          </div>
                          <div>
                            <div class="text-xs text-amber-400">Stats require DB access</div>
                            <div class="text-xs text-muted-foreground">Use phpMyAdmin for details</div>
                          </div>
                        </div>
                      {:else if stats}
                        <div class="flex items-center gap-2.5">
                          <div class="p-2 rounded-lg bg-amber-500/10">
                            <svg class="w-4 h-4 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"/>
                            </svg>
                          </div>
                          <div>
                            <div class="text-sm font-semibold text-foreground">—</div>
                            <div class="text-xs text-muted-foreground">Last backup</div>
                          </div>
                        </div>
                      {/if}
                    </div>
                  {/if}
                </td>
              </tr>
            {/if}
          {/each}
        </tbody>
      </table>
    </div>
  {/if}

</div>

<!-- ════════════════ Bulk Operations Bar ════════════════ -->
{#if selectedIds.size >= 2}
  <div class="fixed bottom-6 left-1/2 -translate-x-1/2 z-50 bg-card border border-border rounded-full px-5 py-2.5 shadow-2xl flex items-center gap-4 text-sm fade-up">
    <span class="text-foreground font-medium">{selectedIds.size} databases selected</span>
    <div class="w-px h-4 bg-border"></div>
    <button
      on:click={() => { showToast('Bulk export is not yet supported', 'error'); }}
      class="text-muted-foreground hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
    >
      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4"/>
      </svg>
      Export All
    </button>
    <button
      on:click={bulkDelete}
      class="text-red-400 hover:text-red-300 inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
    >
      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"/>
      </svg>
      Delete Selected
    </button>
    <button
      on:click={() => { /* bulk change password placeholder */ showToast('Select one database to change password', 'success'); }}
      class="text-muted-foreground hover:text-foreground inline-flex items-center gap-1.5 transition-all duration-150 active:scale-95"
    >
      <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"/>
      </svg>
      Change Password
    </button>
    <button
      on:click={clearSelection}
      class="text-muted-foreground hover:text-foreground transition-all duration-150"
      title="Clear selection"
    >
      <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12"/>
      </svg>
    </button>
  </div>
{/if}

<!-- ════════════════ Create Database Modal ════════════════ -->
{#if showCreateModal}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-label="Create database"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => { if (!createLoading) showCreateModal = false; }}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl max-h-[90vh] overflow-y-auto fade-up">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">Create Database</h2>
        <button
          on:click={() => showCreateModal = false}
          disabled={createLoading}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <form on:submit={handleCreate} class="space-y-5">

        <!-- Type selection -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-2">Database Type</label>
          <div class="grid grid-cols-3 gap-2">
            {#each CREATE_TYPES as type}
              {@const cfg = dbCfg(type)}
              <label class="flex flex-col items-center gap-2 p-3 rounded-xl border cursor-pointer transition-colors
                            {newDb.db_type === type ? 'border-primary bg-primary/5' : 'border-border hover:bg-muted/30'}">
                <input type="radio" bind:group={newDb.db_type} value={type} class="sr-only" />
                <span class="w-8 h-8 rounded-lg flex items-center justify-center text-sm font-bold {cfg.badge}">
                  {cfg.icon}
                </span>
                <span class="text-xs font-medium text-foreground">{cfg.label}</span>
              </label>
            {/each}
          </div>
        </div>

        <!-- Associated site -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="create-site">Associated Site</label>
          <select
            id="create-site"
            bind:value={newDb.site_id}
            required
            class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
          >
            {#each sites as site}
              <option value={site.id}>{site.domain}</option>
            {/each}
          </select>
        </div>

        <!-- DB name -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="create-name">Database Name</label>
          <input
            id="create-name"
            type="text"
            bind:value={newDb.db_name}
            on:input={refreshUsername}
            required
            placeholder="my_database"
            pattern="[a-zA-Z_][a-zA-Z0-9_]*"
            title="Must start with a letter or underscore; letters, numbers and underscores only"
            class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground font-mono placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
          />
          <p class="text-xs text-muted-foreground mt-1">Lowercase letters, numbers, underscores. Max 64 chars.</p>
        </div>

        <!-- Username (auto-generated, editable) -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="create-user">Database Username</label>
          <input
            id="create-user"
            type="text"
            bind:value={newDb.db_user}
            placeholder="Auto-generated"
            class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground font-mono placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
          />
          <p class="text-xs text-muted-foreground mt-1">Will be auto-generated from your site prefix if left blank.</p>
        </div>

        <!-- Password -->
        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="create-pw">Password</label>
          <div class="flex gap-2">
            <div class="relative flex-1">
              <input
                id="create-pw"
                type={showPassword ? 'text' : 'password'}
                bind:value={newDb.db_password}
                placeholder="Enter or generate a password"
                class="h-9 w-full rounded-lg border border-border bg-background px-3 pr-9 text-sm text-foreground font-mono placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
              />
              <button
                type="button"
                on:click={() => showPassword = !showPassword}
                class="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                aria-label={showPassword ? 'Hide password' : 'Show password'}
              >
                {#if showPassword}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.542 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                  </svg>
                {:else}
                  <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                  </svg>
                {/if}
              </button>
            </div>
            <button
              type="button"
              on:click={generatePassword}
              class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 shrink-0 transition-all duration-150 active:scale-95"
            >
              <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Generate
            </button>
          </div>
          <!-- Strength meter -->
          {#if newDb.db_password}
            <div class="mt-2 space-y-1">
              <div class="flex gap-1">
                {#each [1, 2, 3, 4] as i}
                  <div class="h-1 flex-1 rounded-full {passwordStrength >= i ? strengthColor[passwordStrength] : 'bg-muted'}"></div>
                {/each}
              </div>
              <p class="text-xs {passwordStrength >= 3 ? 'text-green-400' : passwordStrength >= 2 ? 'text-amber-400' : 'text-red-400'}">
                {strengthLabel[passwordStrength] ?? ''}
              </p>
            </div>
          {/if}
          <p class="text-xs text-muted-foreground mt-1">Min 12 chars with uppercase, lowercase, number and special char.</p>
        </div>

        {#if createError}
          <div role="alert" class="text-sm text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2.5">
            {createError}
          </div>
        {/if}

        <div class="flex gap-3 pt-1">
          <button
            type="button"
            on:click={() => showCreateModal = false}
            disabled={createLoading}
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={createLoading}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50 transition-all duration-150 active:scale-95"
          >
            {#if createLoading}
              <svg class="w-3.5 h-3.5 animate-spin" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
              </svg>
              Creating…
            {:else}
              Create Database
            {/if}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<!-- ════════════════ Connect Modal — 4-tab code snippets ════════════════ -->
{#if connectTarget}
  {@const db = connectTarget}
  {@const host = db.db_host ?? 'localhost'}
  {@const port = db.db_port ?? defaultPort(db.db_type)}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-label="Connection details"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => connectTarget = null}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-xl shadow-2xl fade-up">
      <div class="flex items-center justify-between mb-5">
        <div>
          <h2 class="text-base font-semibold text-foreground">Connection Details</h2>
          <p class="text-xs text-muted-foreground mt-0.5">{db.db_name}</p>
        </div>
        <button
          on:click={() => connectTarget = null}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <!-- Individual fields row -->
      <div class="grid grid-cols-2 gap-3 mb-5">
        {#each [
          { label: 'Host', value: host, mono: true },
          { label: 'Port', value: String(port), mono: true },
          { label: 'Database', value: db.db_name, mono: true },
          { label: 'Username', value: db.db_user, mono: true },
        ] as field}
          <div>
            <div class="flex items-center justify-between mb-0.5">
              <span class="text-xs text-muted-foreground">{field.label}</span>
              <button
                on:click={() => copyText(field.value)}
                class="text-xs text-muted-foreground hover:text-primary"
                title="Copy"
              >
                <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                    d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2" />
                </svg>
              </button>
            </div>
            <div class="font-mono text-xs bg-background border border-border rounded-lg px-2.5 py-1.5 text-foreground">{field.value}</div>
          </div>
        {/each}

        <!-- Password field with toggle -->
        <div class="col-span-2">
          <div class="flex items-center justify-between mb-0.5">
            <span class="text-xs text-muted-foreground">Password</span>
            <button
              on:click={() => showConnPassword = !showConnPassword}
              class="text-xs text-muted-foreground hover:text-primary inline-flex items-center gap-1"
            >
              {showConnPassword ? 'Hide' : 'Reveal'}
            </button>
          </div>
          <div class="font-mono text-xs bg-background border border-border rounded-lg px-2.5 py-1.5 text-foreground">
            {showConnPassword ? '(see your saved password)' : '••••••••••••'}
          </div>
          <p class="text-xs text-muted-foreground mt-1">Password is not stored in OrbitCP. Use the one you set at creation time.</p>
        </div>
      </div>

      <!-- Code snippet tabs -->
      <div>
        <div class="flex items-center gap-1 mb-3 border-b border-border">
          {#each [
            { key: 'cli',    label: 'CLI' },
            { key: 'php',    label: 'PHP (PDO)' },
            { key: 'python', label: 'Python' },
            { key: 'node',   label: 'Node.js' },
          ] as tab}
            <button
              on:click={() => connTab = tab.key as any}
              class="px-3 py-2 text-xs font-medium transition-colors border-b-2 -mb-px
                {connTab === tab.key
                  ? 'border-primary text-foreground'
                  : 'border-transparent text-muted-foreground hover:text-foreground'}"
            >
              {tab.label}
            </button>
          {/each}
          <div class="ml-auto">
            <button
              on:click={() => copyText(getSnippet(db, connTab))}
              class="h-7 px-2.5 rounded-md border border-border text-xs text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-1.5 mb-1 transition-all duration-150 active:scale-95"
            >
              <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                  d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2" />
              </svg>
              Copy
            </button>
          </div>
        </div>
        <pre class="bg-[#0d1117] rounded-lg p-4 text-xs font-mono overflow-x-auto text-slate-300 leading-relaxed max-h-40">{getSnippet(db, connTab)}</pre>
      </div>
    </div>
  </div>
{/if}

<!-- ════════════════ Export Modal (501 graceful) ════════════════ -->
{#if exportTarget}
  {@const db = exportTarget}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-label="Export database"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => exportTarget = null}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl fade-up">
      <div class="flex items-start gap-4 mb-5">
        <div class="w-10 h-10 rounded-full bg-amber-500/10 flex items-center justify-center shrink-0">
          <svg class="w-5 h-5 text-amber-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
          </svg>
        </div>
        <div>
          <h2 class="text-base font-semibold text-foreground">Export Coming Soon</h2>
          <p class="text-sm text-muted-foreground mt-1">
            Direct SQL export for <strong class="text-foreground">{db.db_name}</strong> is available in the next release.
            In the meantime, use <strong class="text-foreground">phpMyAdmin</strong> to export your database.
          </p>
        </div>
      </div>
      <div class="flex gap-3 justify-end">
        <button
          on:click={() => exportTarget = null}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          Close
        </button>
        <a
          href="/databases/{db.id}"
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
          on:click={() => exportTarget = null}
        >
          Open phpMyAdmin
        </a>
      </div>
    </div>
  </div>
{/if}

<!-- ════════════════ Import Modal (501 graceful) ════════════════ -->
{#if importTarget}
  {@const db = importTarget}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-label="Import database"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => importTarget = null}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl fade-up">
      <div class="flex items-center justify-between mb-5">
        <h2 class="text-base font-semibold text-foreground">Import into {db.db_name}</h2>
        <button
          on:click={() => importTarget = null}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>

      <form on:submit={handleImport} class="space-y-4">
        <div class="rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-3 text-sm text-amber-400">
          SQL import via the API is available in the next release. You can import now using phpMyAdmin.
        </div>

        <div>
          <label class="block text-sm font-medium text-foreground mb-1.5" for="import-file">SQL File</label>
          <input
            id="import-file"
            type="file"
            accept=".sql,.gz"
            bind:files={importFile}
            class="w-full text-sm text-muted-foreground file:mr-3 file:h-8 file:px-3 file:rounded-lg file:border file:border-border file:bg-muted file:text-foreground file:text-xs file:font-medium hover:file:bg-muted/70"
          />
        </div>

        {#if importError}
          <div role="alert" class="text-sm text-amber-400 bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2.5">
            {importError}
            <a href="/databases/{db.id}" class="ml-1 underline hover:no-underline">Open phpMyAdmin instead</a>
          </div>
        {/if}

        <div class="flex gap-3 pt-1">
          <button
            type="button"
            on:click={() => importTarget = null}
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={importLoading || !importFile?.length}
            class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:bg-primary/90 inline-flex items-center gap-2 disabled:opacity-50 transition-all duration-150 active:scale-95"
          >
            {importLoading ? 'Importing…' : 'Import SQL'}
          </button>
        </div>
      </form>
    </div>
  </div>
{/if}

<!-- ════════════════ Delete Confirm Modal ════════════════ -->
{#if deleteTarget}
  {@const db = deleteTarget}
  <div
    class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
    role="dialog"
    aria-modal="true"
    aria-label="Confirm delete database"
  >
    <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
    <div class="fixed inset-0" on:click={() => { deleteTarget = null; deleteConfirmName = ''; }}></div>
    <div class="relative bg-card border border-border rounded-2xl p-6 w-full max-w-md shadow-2xl fade-up">
      <div class="flex items-start gap-4 mb-5">
        <div class="w-10 h-10 rounded-full bg-red-500/10 flex items-center justify-center shrink-0 mt-0.5">
          <svg class="w-5 h-5 text-red-400" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
              d="M12 9v2m0 4h.01M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
          </svg>
        </div>
        <div>
          <h2 class="text-base font-semibold text-foreground">Delete Database?</h2>
          <p class="text-sm text-muted-foreground mt-1">
            This will permanently delete <strong class="text-foreground">{db.db_name}</strong> and all its data.
            This action cannot be undone.
          </p>
        </div>
      </div>

      <div class="mb-4">
        <label class="block text-sm font-medium text-foreground mb-1.5" for="delete-confirm">
          Type <span class="font-mono text-red-400">{db.db_name}</span> to confirm
        </label>
        <input
          id="delete-confirm"
          type="text"
          bind:value={deleteConfirmName}
          placeholder={db.db_name}
          class="h-9 w-full rounded-lg border border-border bg-background px-3 text-sm text-foreground font-mono placeholder:text-muted-foreground focus:outline-none focus:ring-2 focus:ring-red-500/50 focus:border-red-500"
        />
      </div>

      <div class="flex gap-3 justify-end">
        <button
          on:click={() => { deleteTarget = null; deleteConfirmName = ''; }}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-all duration-150 active:scale-95"
        >
          Cancel
        </button>
        <button
          on:click={confirmDelete}
          disabled={deleteConfirmName !== db.db_name}
          class="h-9 px-4 rounded-lg bg-red-500/10 text-red-400 border border-red-500/20 text-sm font-medium hover:bg-red-500/20 inline-flex items-center gap-2 disabled:opacity-50 disabled:cursor-not-allowed transition-all duration-150 active:scale-95"
        >
          Delete Database
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
  .fade-up {
    animation: fadeUp 0.25s ease-out both;
  }
</style>
