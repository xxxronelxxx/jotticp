<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '$api/client';
  import type { Database, Site } from '$api/client';

  // ── Types ──────────────────────────────────────────────────────────────────
  interface TableInfo {
    name: string;
    row_count: number;
    size_bytes: number;
  }

  interface QueryResult {
    columns: string[];
    rows: unknown[][];
    affected_rows: number;
    duration_ms: number;
  }

  interface SavedQuery {
    id: string;
    label: string;
    sql: string;
    savedAt: string;
  }

  // ── State ──────────────────────────────────────────────────────────────────
  let sites: Site[] = [];
  let databases: Database[] = [];
  let selectedSiteId = '';
  let selectedDb: Database | null = null;
  let tables: TableInfo[] = [];
  let selectedTable: TableInfo | null = null;
  let schemas: string[] = [];
  let selectedSchema = 'public';

  // Tree state
  let expandedDbs = new Set<string>();
  let loadingTables = false;

  // Right panel — tabs: browse | query | schema
  let rightTab: 'browse' | 'query' | 'schema' = 'browse';

  // Browse state
  let browseResult: { columns: string[]; rows: unknown[][]; total: number } | null = null;
  let browsePage = 0;
  const PAGE_SIZE = 50;
  let browseLoading = false;

  // Inline row editing
  interface EditingCell {
    rowIdx: number;
    colIdx: number;
    value: string;
    original: string;
  }
  let editingCell: EditingCell | null = null;

  // Query state
  let sql = '';
  let queryResult: QueryResult | null = null;
  let queryLoading = false;
  let queryError = '';
  let importLoading = false;
  let importFileInput: HTMLInputElement;

  // EXPLAIN ANALYZE state
  let explainResult: string | null = null;
  let explainLoading = false;

  // Query history (last 20, persisted in localStorage)
  let queryHistory: string[] = [];
  let historyOpen = false;

  // Saved queries
  let savedQueries: SavedQuery[] = [];
  let saveQueryLabel = '';
  let showSaveDialog = false;

  // DB credentials modal
  let showCredsModal = false;
  let credsDb: Database | null = null;
  let credsPasswordVisible = false;
  let credsPassword = '••••••••••••';
  let credsPasswordLoading = false;
  let credsCopied: string | null = null;

  // Toast
  let toastMessage = '';
  let toastType: 'success' | 'error' = 'success';
  let loadError = '';
  let toastTimer: ReturnType<typeof setTimeout>;

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  onMount(async () => {
    try {
      const stored = localStorage.getItem('orbit-query-history');
      if (stored) queryHistory = JSON.parse(stored);
    } catch { /* ignore */ }
    try {
      const stored = localStorage.getItem('orbit-saved-queries');
      if (stored) savedQueries = JSON.parse(stored);
    } catch { /* ignore */ }

    try {
      [sites, databases] = await Promise.all([
        api.sites.list(),
        api.databases.list(),
      ]);

      const params = new URLSearchParams(window.location.search);
      const urlSiteId = params.get('site_id');
      const urlDbId   = params.get('db_id');
      if (urlSiteId) selectedSiteId = urlSiteId;
      if (urlDbId) {
        const target = databases.find(d => d.id === urlDbId);
        if (target) await selectDb(target);
      }
    } catch (err: unknown) {
      loadError = (err as { message?: string })?.message ?? 'Failed to load databases';
    }
  });

  // ── Reactive filtering ─────────────────────────────────────────────────────
  $: filteredDbs = selectedSiteId
    ? databases.filter(d => d.site_id === selectedSiteId)
    : databases;

  // ── Tree actions ───────────────────────────────────────────────────────────

  async function selectDb(db: Database) {
    selectedDb = db;
    selectedTable = null;
    browseResult = null;
    queryResult = null;
    explainResult = null;
    queryError = '';

    if (expandedDbs.has(db.id)) {
      expandedDbs.delete(db.id);
      expandedDbs = new Set(expandedDbs);
      return;
    }

    loadingTables = true;
    try {
      const [schemaList, tableList] = await Promise.all([
        api.dbmanager.schemas(db.id).catch(() => ['public']),
        api.dbmanager.tables(db.id),
      ]);
      schemas = schemaList;
      if (!schemas.includes(selectedSchema)) selectedSchema = schemas[0] ?? 'public';
      tables = tableList;
      expandedDbs.add(db.id);
      expandedDbs = new Set(expandedDbs);
    } catch {
      showToast('Failed to load tables', 'error');
    } finally {
      loadingTables = false;
    }
  }

  async function selectTable(table: TableInfo) {
    selectedTable = table;
    rightTab = 'browse';
    browsePage = 0;
    editingCell = null;
    await loadRows();
  }

  async function loadRows() {
    if (!selectedDb || !selectedTable) return;
    browseLoading = true;
    try {
      browseResult = await api.dbmanager.rows(
        selectedDb.id,
        selectedTable.name,
        PAGE_SIZE,
        browsePage * PAGE_SIZE,
        selectedSchema,
      );
    } catch {
      showToast('Failed to load rows', 'error');
    } finally {
      browseLoading = false;
    }
  }

  // ── Query history ──────────────────────────────────────────────────────────

  function pushHistory(query: string) {
    queryHistory = [query, ...queryHistory.filter(q => q !== query)].slice(0, 20);
    try { localStorage.setItem('orbit-query-history', JSON.stringify(queryHistory)); } catch { /* ignore */ }
  }

  function restoreFromHistory(query: string) {
    sql = query;
    historyOpen = false;
  }

  // ── Saved queries ──────────────────────────────────────────────────────────

  function saveQuery() {
    if (!sql.trim() || !saveQueryLabel.trim()) return;
    const q: SavedQuery = {
      id: Date.now().toString(),
      label: saveQueryLabel.trim(),
      sql: sql.trim(),
      savedAt: new Date().toISOString(),
    };
    savedQueries = [q, ...savedQueries];
    try { localStorage.setItem('orbit-saved-queries', JSON.stringify(savedQueries)); } catch { /* ignore */ }
    saveQueryLabel = '';
    showSaveDialog = false;
    showToast('Query saved', 'success');
  }

  function deleteSavedQuery(id: string) {
    savedQueries = savedQueries.filter(q => q.id !== id);
    try { localStorage.setItem('orbit-saved-queries', JSON.stringify(savedQueries)); } catch { /* ignore */ }
  }

  function loadSavedQuery(q: SavedQuery) {
    sql = q.sql;
    rightTab = 'query';
  }

  // ── Run query ──────────────────────────────────────────────────────────────

  async function runQuery() {
    if (!selectedDb || !sql.trim()) return;
    queryLoading = true;
    queryError = '';
    queryResult = null;
    explainResult = null;
    try {
      queryResult = await api.dbmanager.query(selectedDb.id, sql.trim());
      pushHistory(sql.trim());
    } catch (err: unknown) {
      const e = err as { message?: string };
      queryError = e.message ?? 'Query failed';
    } finally {
      queryLoading = false;
    }
  }

  // ── EXPLAIN ANALYZE ────────────────────────────────────────────────────────

  async function runExplain() {
    if (!selectedDb || !sql.trim()) return;
    explainLoading = true;
    queryError = '';
    explainResult = null;
    queryResult = null;
    const isPostgres = selectedDb.db_type === 'postgresql';
    const prefix = isPostgres ? 'EXPLAIN ANALYZE ' : 'EXPLAIN ';
    try {
      const res = await api.dbmanager.query(selectedDb.id, prefix + sql.trim());
      explainResult = res.rows.length > 0 ? res.rows.map(r => r[0]).join('\n') : '(no output)';
      pushHistory(sql.trim());
    } catch (err: unknown) {
      const e = err as { message?: string };
      queryError = e.message ?? 'EXPLAIN failed';
    } finally {
      explainLoading = false;
    }
  }

  // ── Inline row editing ─────────────────────────────────────────────────────

  function startEdit(rowIdx: number, colIdx: number, currentValue: unknown) {
    const strVal = currentValue === null ? '' : String(currentValue);
    editingCell = { rowIdx, colIdx, value: strVal, original: strVal };
  }

  function cancelEdit() { editingCell = null; }

  async function commitEdit() {
    if (!editingCell || !selectedDb || !selectedTable || !browseResult) return;
    const { rowIdx, colIdx, value } = editingCell;
    const row = browseResult.rows[rowIdx];
    const columns = browseResult.columns;
    const pkColIdx = columns.findIndex(c => c === 'id') !== -1 ? columns.findIndex(c => c === 'id') : 0;
    const pk = String(row[pkColIdx]);
    const colName = columns[colIdx];
    try {
      await api.dbmanager.updateRow(selectedDb.id, selectedTable.name, pk, { [colName]: value });
      (browseResult.rows[rowIdx] as unknown[])[colIdx] = value;
      browseResult = { ...browseResult };
      showToast('Row updated', 'success');
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Update failed', 'error');
    } finally {
      editingCell = null;
    }
  }

  // ── Export / Import ────────────────────────────────────────────────────────

  function exportCsv() {
    if (!selectedDb || !selectedTable) return;
    window.open(api.dbmanager.exportCsv(selectedDb.id, selectedTable.name), '_blank');
  }

  async function importSql() {
    if (!selectedDb || !importFileInput.files?.[0]) return;
    importLoading = true;
    try {
      const token = typeof localStorage !== 'undefined' ? localStorage.getItem('orbit_access_token') : null;
      await api.dbmanager.importSql(selectedDb.id, importFileInput.files[0], token);
      showToast('SQL imported successfully', 'success');
      importFileInput.value = '';
    } catch (err: unknown) {
      const e = err as { message?: string };
      showToast(e.message ?? 'Import failed', 'error');
    } finally {
      importLoading = false;
    }
  }

  // ── DB Credentials modal ───────────────────────────────────────────────────

  function openCreds(db: Database) {
    credsDb = db;
    credsPasswordVisible = false;
    credsPassword = '••••••••••••';
    credsCopied = null;
    showCredsModal = true;
  }

  async function revealPassword() {
    if (!credsDb) return;
    if (credsPasswordVisible) { credsPasswordVisible = false; credsPassword = '••••••••••••'; return; }
    credsPasswordLoading = true;
    try {
      // Attempt to get stats — password may be revealed via a dedicated endpoint; fall back to placeholder
      credsPasswordVisible = true;
      credsPassword = '(stored server-side — use "Change Password" to update)';
    } catch {
      credsPassword = '(unavailable)';
    } finally {
      credsPasswordLoading = false;
    }
  }

  async function copyToClipboard(text: string, key: string) {
    try {
      await navigator.clipboard.writeText(text);
      credsCopied = key;
      setTimeout(() => { credsCopied = null; }, 2000);
    } catch {
      showToast('Copy failed', 'error');
    }
  }

  function buildConnectionString(db: Database): string {
    if (db.db_type === 'postgresql') {
      return `psql -h ${db.db_host} -p ${db.db_port} -U ${db.db_user} -d ${db.db_name}`;
    }
    if (db.db_type === 'mysql' || db.db_type === 'mariadb') {
      return `mysql -h ${db.db_host} -P ${db.db_port} -u ${db.db_user} -p ${db.db_name}`;
    }
    return `${db.db_host}:${db.db_port}/${db.db_name}`;
  }

  function buildUrlConnectionString(db: Database): string {
    const scheme =
      db.db_type === 'mysql' ? 'mysql' :
      db.db_type === 'mariadb' ? 'mariadb' :
      db.db_type === 'postgresql' ? 'postgresql' :
      db.db_type;
    return `${scheme}://${db.db_user}:****@${db.db_host}:${db.db_port}/${db.db_name}`;
  }

  // ── ERD helpers ────────────────────────────────────────────────────────────

  function getRefTable(fkCol: string): string {
    return fkCol.replace(/_id$/, '') + 's';
  }

  // ── Utility ────────────────────────────────────────────────────────────────

  function showToast(msg: string, type: 'success' | 'error') {
    clearTimeout(toastTimer);
    toastMessage = msg;
    toastType = type;
    toastTimer = setTimeout(() => { toastMessage = ''; }, 4000);
  }

  function formatBytes(b: number): string {
    if (b >= 1_073_741_824) return `${(b / 1_073_741_824).toFixed(1)} GB`;
    if (b >= 1_048_576)     return `${(b / 1_048_576).toFixed(1)} MB`;
    if (b >= 1024)          return `${Math.round(b / 1024)} KB`;
    return `${b} B`;
  }

  // ── SQL formatter ──────────────────────────────────────────────────────────
  function formatSql(input: string): string {
    return input
      .replace(/\b(SELECT|FROM|WHERE|JOIN|LEFT JOIN|INNER JOIN|RIGHT JOIN|OUTER JOIN|ORDER BY|GROUP BY|HAVING|LIMIT|OFFSET|INSERT INTO|INSERT|UPDATE|DELETE|CREATE TABLE|CREATE|DROP TABLE|DROP|ALTER TABLE|ALTER|SET|VALUES|ON|AND|OR|NOT|IN|AS|DISTINCT|COUNT|SUM|AVG|MAX|MIN)\b/gi,
               (m) => '\n' + m.toUpperCase())
      .trim();
  }

  // ── Result table sort ──────────────────────────────────────────────────────
  let sortCol: number | null = null;
  let sortDir: 'asc' | 'desc' = 'asc';

  function toggleSort(colIdx: number) {
    if (sortCol === colIdx) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    } else {
      sortCol = colIdx;
      sortDir = 'asc';
    }
  }

  // ── Cell copy ─────────────────────────────────────────────────────────────
  async function copyCell(value: unknown) {
    try {
      await navigator.clipboard.writeText(value === null ? 'NULL' : String(value));
      showToast('Copied to clipboard', 'success');
    } catch {
      showToast('Copy failed', 'error');
    }
  }

  // ── Export query result as CSV ─────────────────────────────────────────────
  function exportResultCsv() {
    if (!queryResult || queryResult.columns.length === 0) return;
    const header = queryResult.columns.join(',');
    const rows = displayRows.map(row =>
      row.map(cell => {
        const s = cell === null ? '' : String(cell);
        return s.includes(',') || s.includes('"') || s.includes('\n')
          ? '"' + s.replace(/"/g, '""') + '"'
          : s;
      }).join(',')
    );
    const csv = [header, ...rows].join('\n');
    const blob = new Blob([csv], { type: 'text/csv' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `query-result-${Date.now()}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  }

  function dbTypeBadge(type: string): { label: string; color: string } {
    switch (type) {
      case 'mysql':      return { label: 'MySQL',      color: 'bg-blue-500/10 text-blue-500' };
      case 'mariadb':    return { label: 'MariaDB',    color: 'bg-amber-500/10 text-amber-500' };
      case 'postgresql': return { label: 'PostgreSQL', color: 'bg-sky-500/10 text-sky-500' };
      case 'ferretdb':   return { label: 'FerretDB',   color: 'bg-orange-500/10 text-orange-500' };
      case 'surrealdb':  return { label: 'SurrealDB',  color: 'bg-purple-500/10 text-purple-500' };
      default:           return { label: type,          color: 'bg-muted text-muted-foreground' };
    }
  }

  $: totalBrowsePages = browseResult ? Math.ceil(browseResult.total / PAGE_SIZE) : 0;
  $: rawDisplayRows = queryResult ? queryResult.rows.slice(0, 1000) : (browseResult?.rows ?? []);
  $: displayRows = sortCol !== null
    ? [...rawDisplayRows].sort((a, b) => {
        const av = (a as unknown[])[sortCol!];
        const bv = (b as unknown[])[sortCol!];
        const cmp = av === null ? -1 : bv === null ? 1
          : typeof av === 'number' && typeof bv === 'number' ? av - bv
          : String(av).localeCompare(String(bv));
        return sortDir === 'asc' ? cmp : -cmp;
      })
    : rawDisplayRows;
  $: displayColumns = queryResult?.columns ?? browseResult?.columns ?? [];
  // Top tables by size for visualization
  $: topTables = [...tables].sort((a, b) => b.size_bytes - a.size_bytes).slice(0, 10);
  $: maxTableSize = topTables.length > 0 ? topTables[0].size_bytes : 1;
</script>

<svelte:head>
  <title>DB Manager — OrbitCP</title>
</svelte:head>

<!-- svelte-ignore a11y-no-static-element-interactions -->
<svelte:window on:keydown={e => {
  const ta = document.querySelector('textarea[data-query-editor]');
  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter' && document.activeElement === ta) {
    e.preventDefault();
    if (!queryLoading && sql.trim()) runQuery();
  }
}} />

<div class="flex h-[calc(100vh-4rem)] overflow-hidden">

  <!-- ── Left sidebar ─────────────────────────────────────────────────────── -->
  <aside class="w-64 shrink-0 flex flex-col bg-card border-r border-border overflow-hidden">

    <!-- Sidebar header -->
    <div class="flex items-center gap-2.5 px-3 py-3 border-b border-border shrink-0">
      <div class="w-7 h-7 rounded-lg flex items-center justify-center shrink-0"
           style="background:rgba(59,130,246,.1)">
        <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="#3b82f6" stroke-width="1.75">
          <ellipse cx="12" cy="5" rx="9" ry="3"/>
          <path d="M3 5v14c0 1.66 4.03 3 9 3s9-1.34 9-3V5"/>
          <path d="M3 12c0 1.66 4.03 3 9 3s9-1.34 9-3"/>
        </svg>
      </div>
      <span class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Databases</span>
    </div>

    <!-- Site filter -->
    <div class="px-3 py-2.5 border-b border-border shrink-0">
      <select
        bind:value={selectedSiteId}
        class="w-full h-8 px-2.5 rounded-lg border border-border bg-background text-xs
               text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary"
      >
        <option value="">All sites</option>
        {#each sites as site}
          <option value={site.id}>{site.domain}</option>
        {/each}
      </select>
    </div>

    <!-- DB + table tree -->
    <div class="flex-1 overflow-y-auto py-1">
      {#if filteredDbs.length === 0}
        <div class="flex flex-col items-center justify-center py-10 px-4 text-center">
          <svg class="w-8 h-8 text-muted-foreground mb-2 opacity-40" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1">
            <ellipse cx="12" cy="5" rx="9" ry="3"/>
            <path d="M3 5v14c0 1.66 4.03 3 9 3s9-1.34 9-3V5"/>
            <path d="M3 12c0 1.66 4.03 3 9 3s9-1.34 9-3"/>
          </svg>
          <p class="text-xs text-muted-foreground">No databases found.</p>
        </div>
      {:else}
        {#each filteredDbs as db (db.id)}
          <!-- DB node -->
          <button
            type="button"
            on:click={() => selectDb(db)}
            class="w-full flex items-center gap-1.5 px-3 py-2 text-left text-xs font-medium
                   hover:bg-muted/50 transition-colors
                   {selectedDb?.id === db.id ? 'bg-primary/10 text-primary' : 'text-foreground'}"
          >
            <svg class="w-3 h-3 shrink-0 transition-transform {expandedDbs.has(db.id) ? 'rotate-90' : ''}"
                 fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
              <path stroke-linecap="round" stroke-linejoin="round" d="M9 18l6-6-6-6" />
            </svg>
            <svg class="w-3.5 h-3.5 shrink-0 text-blue-500" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="1.5"
                d="M4 7c0-1.66 3.13-3 8-3s8 1.34 8 3v10c0 1.66-3.13 3-8 3s-8-1.34-8-3V7z M4 12c0 1.66 3.13 3 8 3s8-1.34 8-3 M4 7c0 1.66 3.13 3 8 3s8-1.34 8-3" />
            </svg>
            <span class="truncate font-mono">{db.db_name}</span>
            <span class="ml-auto text-muted-foreground text-[10px] shrink-0 capitalize">{db.db_type}</span>
          </button>

          <!-- Table nodes -->
          {#if expandedDbs.has(db.id)}
            {#if loadingTables && selectedDb?.id === db.id}
              <div class="pl-8 py-2">
                <div class="flex items-center gap-2">
                  <div class="w-3 h-3 rounded-full border border-border border-t-primary animate-spin"></div>
                  <span class="text-xs text-muted-foreground">Loading…</span>
                </div>
              </div>
            {:else}
              {#each tables as table}
                <button
                  type="button"
                  on:click={() => { selectedDb = db; selectTable(table); }}
                  class="w-full flex items-center gap-1.5 pl-8 pr-3 py-1.5 text-left text-xs
                         hover:bg-muted/50 transition-colors
                         {selectedTable?.name === table.name && selectedDb?.id === db.id
                           ? 'bg-primary/10 text-primary font-medium'
                           : 'text-muted-foreground hover:text-foreground'}"
                >
                  <svg class="w-3 h-3 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
                    <path stroke-linecap="round" stroke-linejoin="round"
                      d="M3 10h18M3 14h18M10 6h4M10 18h4M3 6a1 1 0 011-1h16a1 1 0 011 1v12a1 1 0 01-1 1H4a1 1 0 01-1-1V6z" />
                  </svg>
                  <span class="truncate font-mono">{table.name}</span>
                  <span class="ml-auto text-[10px] shrink-0">{table.row_count.toLocaleString()}</span>
                </button>
              {/each}
            {/if}
          {/if}
        {/each}
      {/if}
    </div>

    <!-- Saved queries section -->
    {#if savedQueries.length > 0}
      <div class="border-t border-border shrink-0">
        <div class="px-3 py-2 flex items-center justify-between">
          <span class="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider">Saved Queries</span>
        </div>
        <div class="max-h-40 overflow-y-auto">
          {#each savedQueries as sq}
            <div class="flex items-center gap-1 px-2 py-1 hover:bg-muted/30 group">
              <button
                type="button"
                on:click={() => loadSavedQuery(sq)}
                class="flex-1 text-left text-xs text-foreground truncate font-mono"
                title={sq.sql}
              >
                {sq.label}
              </button>
              <button
                type="button"
                on:click={() => deleteSavedQuery(sq.id)}
                class="opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive
                       transition-opacity p-0.5 rounded"
                aria-label="Delete saved query"
              >
                <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  </aside>

  <!-- ── Right panel ────────────────────────────────────────────────────────── -->
  <div class="flex-1 flex flex-col overflow-hidden bg-background">

    {#if !selectedDb}
      <!-- ── No DB selected: launcher / overview ─────────────────────────── -->
      <div class="flex-1 overflow-y-auto">

        <!-- Page header -->
        <div class="flex items-center justify-between px-5 py-4 border-b border-border bg-card/50 shrink-0">
          <div class="flex items-center gap-3">
            <div class="w-9 h-9 rounded-xl flex items-center justify-center"
                 style="background:rgba(59,130,246,.1)">
              <svg class="w-4.5 h-4.5" fill="none" viewBox="0 0 24 24" stroke="#3b82f6" stroke-width="1.75">
                <ellipse cx="12" cy="5" rx="9" ry="3"/>
                <path d="M3 5v14c0 1.66 4.03 3 9 3s9-1.34 9-3V5"/>
                <path d="M3 12c0 1.66 4.03 3 9 3s9-1.34 9-3"/>
              </svg>
            </div>
            <div>
              <h1 class="text-base font-semibold text-foreground">Database Manager</h1>
              <p class="text-xs text-muted-foreground">{databases.length} database{databases.length !== 1 ? 's' : ''} available</p>
            </div>
          </div>
          <button
            type="button"
            on:click={() => filteredDbs[0] && openCreds(filteredDbs[0])}
            disabled={filteredDbs.length === 0}
            class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                   hover:bg-muted hover:text-foreground inline-flex items-center gap-2 transition-colors
                   disabled:opacity-40 disabled:pointer-events-none"
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <rect x="3" y="11" width="18" height="11" rx="2" stroke-linecap="round" stroke-linejoin="round"/>
              <path stroke-linecap="round" stroke-linejoin="round" d="M7 11V7a5 5 0 0110 0v4"/>
            </svg>
            Connection Info
          </button>
        </div>

        <!-- Body -->
        <div class="p-5 space-y-6 max-w-5xl">
  <!-- ── Load error banner ──────────────────────────────────────────────── -->
  {#if loadError}
    <div class="flex items-center gap-3 px-4 py-3 rounded-xl
                bg-red-500/10 border border-red-500/20 text-sm text-red-400">
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



          {#if databases.length === 0}
            <!-- Empty state -->
            <div class="bg-card border border-border rounded-xl p-10 flex flex-col items-center justify-center text-center">
              <div class="w-14 h-14 rounded-2xl bg-muted flex items-center justify-center mb-4">
                <svg class="w-7 h-7 text-muted-foreground" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1">
                  <ellipse cx="12" cy="5" rx="9" ry="3"/>
                  <path d="M3 5v14c0 1.66 4.03 3 9 3s9-1.34 9-3V5"/>
                  <path d="M3 12c0 1.66 4.03 3 9 3s9-1.34 9-3"/>
                </svg>
              </div>
              <h3 class="text-base font-semibold text-foreground mb-1">No databases yet</h3>
              <p class="text-sm text-muted-foreground mb-5">Create a database from the Sites page.</p>
              <a href="/sites"
                 class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                        hover:bg-primary/90 inline-flex items-center gap-2 transition-colors">
                <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v16m8-8H4"/>
                </svg>
                Go to Sites
              </a>
            </div>

          {:else}

            <!-- Quick Actions card -->
            <div class="bg-card border border-border rounded-xl overflow-hidden">
              <div class="px-5 py-3.5 border-b border-border">
                <h2 class="text-sm font-semibold text-foreground">Quick Actions</h2>
                <p class="text-xs text-muted-foreground mt-0.5">Common database management tasks</p>
              </div>
              <div class="p-5 grid grid-cols-1 sm:grid-cols-3 gap-3">
                <!-- Connection Info -->
                <button
                  type="button"
                  on:click={() => filteredDbs[0] && openCreds(filteredDbs[0])}
                  disabled={filteredDbs.length === 0}
                  class="flex flex-col items-start gap-2.5 p-4 rounded-xl border border-border
                         hover:border-blue-500/40 hover:bg-blue-500/5 transition-all group text-left
                         disabled:opacity-50 disabled:pointer-events-none"
                >
                  <div class="w-9 h-9 rounded-lg flex items-center justify-center"
                       style="background:rgba(59,130,246,.12)">
                    <svg class="w-4.5 h-4.5" fill="none" viewBox="0 0 24 24" stroke="#3b82f6" stroke-width="1.75">
                      <rect x="3" y="11" width="18" height="11" rx="2" stroke-linecap="round" stroke-linejoin="round"/>
                      <path stroke-linecap="round" stroke-linejoin="round" d="M7 11V7a5 5 0 0110 0v4"/>
                    </svg>
                  </div>
                  <div>
                    <p class="text-sm font-medium text-foreground group-hover:text-blue-500 transition-colors">Connection Info</p>
                    <p class="text-xs text-muted-foreground mt-0.5">View host, credentials and connection strings</p>
                  </div>
                </button>

                <!-- Select DB to query -->
                <button
                  type="button"
                  on:click={() => filteredDbs[0] && selectDb(filteredDbs[0])}
                  disabled={filteredDbs.length === 0}
                  class="flex flex-col items-start gap-2.5 p-4 rounded-xl border border-border
                         hover:border-primary/40 hover:bg-primary/5 transition-all group text-left
                         disabled:opacity-50 disabled:pointer-events-none"
                >
                  <div class="w-9 h-9 rounded-lg flex items-center justify-center"
                       style="background:rgba(99,102,241,.1)">
                    <svg class="w-4.5 h-4.5" fill="none" viewBox="0 0 24 24" stroke="#6366f1" stroke-width="1.75">
                      <path stroke-linecap="round" stroke-linejoin="round" d="M8 9l-4 3 4 3M16 9l4 3-4 3M12 5l-2 14"/>
                    </svg>
                  </div>
                  <div>
                    <p class="text-sm font-medium text-foreground group-hover:text-primary transition-colors">SQL Query Runner</p>
                    <p class="text-xs text-muted-foreground mt-0.5">Run queries against your database</p>
                  </div>
                </button>

                <!-- View credentials -->
                <button
                  type="button"
                  on:click={() => filteredDbs[0] && openCreds(filteredDbs[0])}
                  disabled={filteredDbs.length === 0}
                  class="flex flex-col items-start gap-2.5 p-4 rounded-xl border border-border
                         hover:border-emerald-500/40 hover:bg-emerald-500/5 transition-all group text-left
                         disabled:opacity-50 disabled:pointer-events-none"
                >
                  <div class="w-9 h-9 rounded-lg flex items-center justify-center"
                       style="background:rgba(16,185,129,.1)">
                    <svg class="w-4.5 h-4.5" fill="none" viewBox="0 0 24 24" stroke="#10b981" stroke-width="1.75">
                      <rect x="3" y="11" width="18" height="11" rx="2" stroke-linecap="round" stroke-linejoin="round"/>
                      <path stroke-linecap="round" stroke-linejoin="round" d="M7 11V7a5 5 0 0110 0v4"/>
                    </svg>
                  </div>
                  <div>
                    <p class="text-sm font-medium text-foreground group-hover:text-emerald-500 transition-colors">Database Credentials</p>
                    <p class="text-xs text-muted-foreground mt-0.5">View connection info and credentials</p>
                  </div>
                </button>
              </div>
            </div>

            <!-- Your Databases table -->
            <div class="bg-card border border-border rounded-xl overflow-hidden">
              <div class="px-5 py-3.5 border-b border-border flex items-center justify-between">
                <div>
                  <h2 class="text-sm font-semibold text-foreground">Your Databases</h2>
                  <p class="text-xs text-muted-foreground mt-0.5">{filteredDbs.length} database{filteredDbs.length !== 1 ? 's' : ''}</p>
                </div>
              </div>
              <div class="overflow-x-auto">
                <table class="w-full text-sm">
                  <thead>
                    <tr class="border-b border-border bg-muted/30">
                      <th class="px-5 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">Name</th>
                      <th class="px-5 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">Type</th>
                      <th class="px-5 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">Size</th>
                      <th class="px-5 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">User</th>
                      <th class="px-5 py-2.5 text-left text-xs font-semibold text-muted-foreground uppercase tracking-wider">Host</th>
                      <th class="px-5 py-2.5 text-right text-xs font-semibold text-muted-foreground uppercase tracking-wider">Actions</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-border">
                    {#each filteredDbs as db (db.id)}
                      {@const badge = dbTypeBadge(db.db_type)}
                      <tr class="hover:bg-muted/20 transition-colors">
                        <td class="px-5 py-3">
                          <span class="font-mono text-sm font-medium text-foreground">{db.db_name}</span>
                        </td>
                        <td class="px-5 py-3">
                          <span class="inline-flex items-center px-2 py-0.5 rounded-md text-xs font-medium {badge.color}">
                            {badge.label}
                          </span>
                        </td>
                        <td class="px-5 py-3 text-sm text-muted-foreground">
                          {db.size_mb == null ? '—' : (db.size_mb >= 1024 ? `${(db.size_mb / 1024).toFixed(1)} GB` : `${db.size_mb} MB`)}
                        </td>
                        <td class="px-5 py-3">
                          <span class="font-mono text-xs text-muted-foreground">{db.db_user}</span>
                        </td>
                        <td class="px-5 py-3">
                          <span class="font-mono text-xs text-muted-foreground">{db.db_host ? `${db.db_host}:${db.db_port ?? ''}` : '—'}</span>
                        </td>
                        <td class="px-5 py-3">
                          <div class="flex items-center justify-end gap-1.5">
                            <button
                              type="button"
                              on:click={() => selectDb(db)}
                              class="h-7 px-2.5 rounded-md text-xs font-medium text-muted-foreground
                                     hover:bg-muted hover:text-foreground transition-colors border border-border"
                              title="Open SQL editor"
                            >
                              Query
                            </button>
                            <button
                              type="button"
                              on:click={() => openCreds(db)}
                              class="h-7 px-2.5 rounded-md text-xs font-medium text-muted-foreground
                                     hover:bg-muted hover:text-foreground transition-colors border border-border"
                              title="View credentials"
                            >
                              Credentials
                            </button>
                          </div>
                        </td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            </div>

            <!-- Database size visualization -->
            {#if filteredDbs.some(d => d.size_mb > 0)}
              <div class="bg-card border border-border rounded-xl overflow-hidden">
                <div class="px-5 py-3.5 border-b border-border">
                  <h2 class="text-sm font-semibold text-foreground">Storage Overview</h2>
                  <p class="text-xs text-muted-foreground mt-0.5">Top databases by size</p>
                </div>
                <div class="p-5 space-y-2">
                  {#each [...filteredDbs].sort((a, b) => b.size_mb - a.size_mb).slice(0, 8) as db}
                    {@const maxMb = Math.max(...filteredDbs.map(d => d.size_mb), 1)}
                    <div class="flex items-center gap-3 text-xs">
                      <span class="w-36 truncate text-muted-foreground font-mono">{db.db_name}</span>
                      <div class="flex-1 bg-muted rounded-full h-1.5">
                        <div class="bg-primary/60 h-1.5 rounded-full transition-all duration-500"
                             style="width: {(db.size_mb / maxMb) * 100}%"></div>
                      </div>
                      <span class="w-16 text-right text-muted-foreground shrink-0">
                        {db.size_mb >= 1024 ? `${(db.size_mb / 1024).toFixed(1)} GB` : `${db.size_mb} MB`}
                      </span>
                    </div>
                  {/each}
                </div>
              </div>
            {/if}

          {/if}
        </div>
      </div>

    {:else}
      <!-- ── DB selected: full SQL manager ─────────────────────────────────── -->

      <!-- Header bar -->
      <div class="flex items-center justify-between px-4 py-2.5 border-b border-border bg-card/50 shrink-0 gap-3">
        <div class="flex items-center gap-2.5 min-w-0">
          <!-- Back to overview -->
          <button
            type="button"
            on:click={() => { selectedDb = null; selectedTable = null; browseResult = null; queryResult = null; }}
            class="h-7 w-7 rounded-md flex items-center justify-center text-muted-foreground
                   hover:bg-muted hover:text-foreground transition-colors shrink-0"
            title="Back to overview"
          >
            <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
              <path stroke-linecap="round" stroke-linejoin="round" d="M15 19l-7-7 7-7"/>
            </svg>
          </button>

          <div class="flex items-center gap-1.5">
            <svg class="w-4 h-4 text-blue-500 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.5">
              <ellipse cx="12" cy="5" rx="9" ry="3"/>
              <path d="M3 5v14c0 1.66 4.03 3 9 3s9-1.34 9-3V5"/>
              <path d="M3 12c0 1.66 4.03 3 9 3s9-1.34 9-3"/>
            </svg>
            <span class="text-sm font-medium text-foreground font-mono truncate">
              {selectedDb.db_name}{selectedTable ? ` / ${selectedTable.name}` : ''}
            </span>
            <span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium {dbTypeBadge(selectedDb.db_type).color}">
              {dbTypeBadge(selectedDb.db_type).label}
            </span>
            {#if selectedTable}
              <span class="text-xs text-muted-foreground">
                {selectedTable.row_count.toLocaleString()} rows · {formatBytes(selectedTable.size_bytes)}
              </span>
            {/if}
          </div>
        </div>

        <div class="flex items-center gap-1.5 shrink-0">
          <!-- Sub-tabs -->
          {#each [['browse','Browse'],['query','Query'],['schema','Schema']] as [tab, label]}
            <button
              type="button"
              on:click={() => rightTab = tab as 'browse' | 'query' | 'schema'}
              class="h-7 px-3 rounded-md text-xs font-medium transition-colors
                     {rightTab === tab ? 'bg-primary text-primary-foreground' : 'text-muted-foreground hover:bg-muted hover:text-foreground'}"
            >
              {label}
            </button>
          {/each}

          {#if selectedTable}
            <div class="w-px h-4 bg-border mx-0.5"></div>
            <button
              type="button"
              on:click={exportCsv}
              class="h-7 px-2.5 rounded-md text-xs font-medium text-muted-foreground hover:bg-muted transition-colors"
            >
              Export CSV
            </button>
            <label class="h-7 px-2.5 rounded-md text-xs font-medium text-muted-foreground hover:bg-muted
                          transition-colors cursor-pointer inline-flex items-center
                          {importLoading ? 'opacity-50 pointer-events-none' : ''}">
              {importLoading ? 'Importing…' : 'Import SQL'}
              <input
                type="file"
                accept=".sql"
                bind:this={importFileInput}
                on:change={importSql}
                class="sr-only"
              />
            </label>
          {/if}

          <div class="w-px h-4 bg-border mx-0.5"></div>
          <button
            type="button"
            on:click={() => openCreds(selectedDb!)}
            class="h-7 px-2.5 rounded-md text-xs font-medium text-muted-foreground hover:bg-muted transition-colors
                   inline-flex items-center gap-1.5"
          >
            <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
              <rect x="3" y="11" width="18" height="11" rx="2"/>
              <path d="M7 11V7a5 5 0 0110 0v4"/>
            </svg>
            Credentials
          </button>
        </div>
      </div>

      <!-- ── Browse tab ─────────────────────────────────────────────────────── -->
      {#if rightTab === 'browse'}
        {#if !selectedTable}
          <div class="flex-1 flex items-center justify-center text-center p-8">
            <div>
              <svg class="w-10 h-10 mx-auto text-muted-foreground mb-3 opacity-40" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1">
                <path stroke-linecap="round" stroke-linejoin="round" d="M3 10h18M3 14h18M10 6h4M10 18h4M3 6a1 1 0 011-1h16a1 1 0 011 1v12a1 1 0 01-1 1H4a1 1 0 01-1-1V6z" />
              </svg>
              <p class="text-sm font-medium text-foreground mb-1">No table selected</p>
              <p class="text-xs text-muted-foreground">Choose a table from the left panel to browse rows.</p>
            </div>
          </div>
        {:else if browseLoading}
          <div class="flex-1 flex items-center justify-center">
            <div class="flex flex-col items-center gap-3">
              <div class="w-6 h-6 rounded-full border-2 border-border border-t-primary animate-spin"></div>
              <span class="text-xs text-muted-foreground">Loading rows…</span>
            </div>
          </div>
        {:else if browseResult}
          <div class="flex-1 overflow-auto">
            <table class="min-w-full text-xs border-collapse">
              <thead class="bg-muted/40 sticky top-0 z-10">
                <tr>
                  {#each browseResult.columns as col}
                    <th class="px-3 py-2 text-left font-semibold text-muted-foreground border-b border-border whitespace-nowrap uppercase tracking-wider text-[10px]">
                      {col}
                    </th>
                  {/each}
                </tr>
              </thead>
              <tbody class="divide-y divide-border">
                {#each browseResult.rows as row, rowIdx}
                  <tr class="hover:bg-muted/20 transition-colors">
                    {#each row as cell, colIdx}
                      <td
                        class="px-3 py-2 font-mono text-foreground whitespace-nowrap max-w-xs truncate cursor-pointer"
                        on:dblclick={() => startEdit(rowIdx, colIdx, cell)}
                        title="Double-click to edit"
                      >
                        {#if editingCell && editingCell.rowIdx === rowIdx && editingCell.colIdx === colIdx}
                          <div class="flex items-center gap-1">
                            <input
                              type="text"
                              bind:value={editingCell.value}
                              class="h-6 px-1.5 rounded border border-primary bg-background text-foreground
                                     font-mono text-xs focus:outline-none focus:ring-1 focus:ring-primary/50 w-32"
                              on:keydown={e => {
                                if (e.key === 'Enter') commitEdit();
                                if (e.key === 'Escape') cancelEdit();
                              }}
                              autofocus
                            />
                            <button type="button" on:click={commitEdit}
                                    class="h-5 px-1.5 rounded bg-primary text-primary-foreground text-[10px] font-medium">
                              Save
                            </button>
                            <button type="button" on:click={cancelEdit}
                                    class="h-5 px-1.5 rounded bg-muted text-muted-foreground text-[10px] hover:bg-muted/80">
                              ✕
                            </button>
                          </div>
                        {:else if cell === null}
                          <span class="text-muted-foreground italic opacity-60">NULL</span>
                        {:else}
                          {String(cell)}
                        {/if}
                      </td>
                    {/each}
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>

          <!-- Pagination -->
          <div class="flex items-center justify-between px-4 py-2.5 border-t border-border bg-card/50 shrink-0">
            {#if totalBrowsePages > 1}
              <span class="text-xs text-muted-foreground">
                Page {browsePage + 1} of {totalBrowsePages}
                · {browseResult.total.toLocaleString()} total rows
                · <span class="text-primary/70">double-click cell to edit</span>
              </span>
              <div class="flex items-center gap-1.5">
                <button
                  on:click={() => { browsePage = Math.max(0, browsePage - 1); loadRows(); }}
                  disabled={browsePage === 0}
                  class="h-7 px-3 rounded-md text-xs text-muted-foreground border border-border
                         hover:bg-muted disabled:opacity-40 transition-colors"
                >
                  Previous
                </button>
                <button
                  on:click={() => { browsePage = Math.min(totalBrowsePages - 1, browsePage + 1); loadRows(); }}
                  disabled={browsePage >= totalBrowsePages - 1}
                  class="h-7 px-3 rounded-md text-xs text-muted-foreground border border-border
                         hover:bg-muted disabled:opacity-40 transition-colors"
                >
                  Next
                </button>
              </div>
            {:else}
              <span class="text-xs text-muted-foreground">
                {browseResult.total.toLocaleString()} row{browseResult.total !== 1 ? 's' : ''}
                · <span class="text-primary/70">double-click any cell to edit</span>
              </span>
              <span></span>
            {/if}
          </div>
        {/if}

      <!-- ── Query tab ─────────────────────────────────────────────────────── -->
      {:else if rightTab === 'query'}
        <div class="flex flex-col flex-1 overflow-hidden">
          <!-- SQL editor area -->
          <div class="p-4 border-b border-border shrink-0 space-y-3">

            <!-- History dropdown -->
            {#if queryHistory.length > 0}
              <div class="relative">
                <button
                  type="button"
                  on:click={() => historyOpen = !historyOpen}
                  class="h-7 px-3 rounded-lg text-xs font-medium text-muted-foreground
                         hover:bg-muted transition-colors flex items-center gap-1.5 border border-border"
                >
                  <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" />
                  </svg>
                  History ({queryHistory.length})
                  <svg class="w-3 h-3 transition-transform {historyOpen ? 'rotate-180' : ''}" fill="none" viewBox="0 0 16 16" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M4 6l4 4 4-4"/>
                  </svg>
                </button>
                {#if historyOpen}
                  <!-- svelte-ignore a11y-click-events-have-key-events -->
                  <!-- svelte-ignore a11y-no-static-element-interactions -->
                  <div class="absolute top-9 left-0 z-20 w-96 max-h-52 overflow-y-auto
                              bg-card border border-border rounded-xl shadow-2xl py-1"
                       on:click|stopPropagation>
                    {#each queryHistory as hq, i}
                      <button
                        type="button"
                        on:click={() => restoreFromHistory(hq)}
                        class="w-full text-left px-3 py-2 text-xs font-mono text-foreground
                               hover:bg-muted/60 transition-colors border-b border-border/40 last:border-0 truncate"
                        title={hq}
                      >
                        <span class="text-muted-foreground mr-2">{i + 1}.</span>{hq}
                      </button>
                    {/each}
                  </div>
                  <!-- svelte-ignore a11y-click-events-have-key-events -->
                  <!-- svelte-ignore a11y-no-static-element-interactions -->
                  <div class="fixed inset-0 z-10" on:click={() => historyOpen = false}></div>
                {/if}
              </div>
            {/if}

            <textarea
              bind:value={sql}
              placeholder="SELECT * FROM your_table LIMIT 100;"
              rows="6"
              data-query-editor
              class="w-full rounded-lg border border-border bg-background px-3 py-2.5 text-xs font-mono
                     text-foreground focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary
                     resize-y placeholder:text-muted-foreground"
            ></textarea>

            <div class="flex items-center gap-2 flex-wrap">
              <button
                type="button"
                on:click={runQuery}
                disabled={queryLoading || !sql.trim()}
                class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                       hover:bg-primary/90 transition-colors disabled:opacity-50 inline-flex items-center gap-2"
              >
                {#if queryLoading}
                  <div class="w-3.5 h-3.5 rounded-full border-2 border-primary-foreground/30 border-t-primary-foreground animate-spin"></div>
                  Running…
                {:else}
                  <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M5.25 5.653c0-.856.917-1.398 1.667-.986l11.54 6.348a1.125 1.125 0 010 1.971l-11.54 6.347a1.125 1.125 0 01-1.667-.985V5.653z"/>
                  </svg>
                  Run Query
                  <span class="text-[9px] opacity-50 font-normal">Ctrl+↵</span>
                {/if}
              </button>

              <button
                type="button"
                on:click={runExplain}
                disabled={explainLoading || !sql.trim()}
                class="h-9 px-4 rounded-lg border border-border bg-background text-sm font-medium text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50 inline-flex items-center gap-2"
                title="Prefix with EXPLAIN ANALYZE (PostgreSQL) or EXPLAIN (MySQL)"
              >
                {explainLoading ? 'Analyzing…' : 'EXPLAIN ANALYZE'}
              </button>

              <button
                type="button"
                on:click={() => { sql = formatSql(sql); }}
                disabled={!sql.trim()}
                class="h-9 px-3 rounded-lg border border-border bg-background text-sm font-medium text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
                title="Format SQL keywords"
              >
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M10 20l4-16m4 4l4 4-4 4M6 16l-4-4 4-4" />
                </svg>
                Format SQL
              </button>

              <button
                type="button"
                on:click={() => showSaveDialog = !showSaveDialog}
                disabled={!sql.trim()}
                class="h-9 px-3 rounded-lg border border-border bg-background text-sm font-medium text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-colors disabled:opacity-50 inline-flex items-center gap-1.5"
              >
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M17 3H5a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2V7l-4-4z"/>
                  <path stroke-linecap="round" stroke-linejoin="round" d="M17 3v4H7V3M12 17a2 2 0 100-4 2 2 0 000 4z"/>
                </svg>
                Save
              </button>

              <span class="text-xs text-muted-foreground">Results limited to 1000 rows</span>

              {#if queryResult}
                <div class="flex items-center gap-2 ml-auto">
                  <span class="text-xs text-muted-foreground">
                    {queryResult.rows.length.toLocaleString()} rows in {queryResult.duration_ms}ms
                    {#if queryResult.affected_rows > 0}· {queryResult.affected_rows} affected{/if}
                  </span>
                  {#if queryResult.columns.length > 0}
                    <button
                      type="button"
                      on:click={exportResultCsv}
                      class="h-7 px-2.5 rounded-md text-xs font-medium text-muted-foreground hover:bg-muted
                             transition-colors border border-border inline-flex items-center gap-1"
                    >
                      <svg class="w-3 h-3" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                        <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                      </svg>
                      CSV
                    </button>
                  {/if}
                </div>
              {/if}
            </div>

            <!-- Save query dialog -->
            {#if showSaveDialog}
              <div class="flex items-center gap-2 mt-1 p-3 rounded-lg bg-muted/30 border border-border">
                <input
                  type="text"
                  bind:value={saveQueryLabel}
                  placeholder="Query label…"
                  class="h-9 flex-1 px-3 rounded-lg border border-border bg-background text-sm text-foreground
                         focus:outline-none focus:ring-2 focus:ring-primary/50 focus:border-primary
                         placeholder:text-muted-foreground"
                  on:keydown={e => e.key === 'Enter' && saveQuery()}
                />
                <button type="button" on:click={saveQuery}
                        disabled={!saveQueryLabel.trim()}
                        class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                               hover:bg-primary/90 disabled:opacity-50 transition-colors">
                  Save
                </button>
                <button type="button" on:click={() => showSaveDialog = false}
                        class="h-9 px-3 rounded-lg border border-border text-sm text-muted-foreground
                               hover:bg-muted transition-colors">
                  Cancel
                </button>
              </div>
            {/if}

            {#if queryError}
              <div class="bg-destructive/10 border border-destructive/20 rounded-lg px-3 py-2.5 text-xs text-destructive font-mono flex items-start gap-2">
                <svg class="w-3.5 h-3.5 mt-0.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <circle cx="12" cy="12" r="10"/>
                  <path d="M12 8v4M12 16h.01"/>
                </svg>
                {queryError}
              </div>
            {/if}
          </div>

          <!-- EXPLAIN result -->
          {#if explainResult}
            <div class="flex-1 overflow-auto p-4">
              <p class="text-xs font-semibold text-muted-foreground mb-2 uppercase tracking-wider">EXPLAIN ANALYZE output:</p>
              <pre class="text-xs font-mono text-foreground bg-muted/20 rounded-xl p-4 overflow-auto
                          whitespace-pre-wrap border border-border leading-relaxed">{explainResult}</pre>
            </div>

          <!-- Query results table -->
          {:else if queryResult && queryResult.columns.length > 0}
            <div class="flex-1 overflow-auto">
              <table class="min-w-full text-xs border-collapse">
                <thead class="bg-muted/40 sticky top-0 z-10">
                  <tr>
                    {#each queryResult.columns as col, ci}
                      <th
                        class="px-3 py-2 text-left font-semibold text-muted-foreground border-b border-border
                               whitespace-nowrap uppercase tracking-wider text-[10px] cursor-pointer
                               hover:bg-muted/60 select-none transition-colors"
                        on:click={() => toggleSort(ci)}
                      >
                        <span class="inline-flex items-center gap-1">
                          {col}
                          {#if sortCol === ci}
                            <svg class="w-2.5 h-2.5 shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                              {#if sortDir === 'asc'}
                                <path stroke-linecap="round" stroke-linejoin="round" d="M5 15l7-7 7 7" />
                              {:else}
                                <path stroke-linecap="round" stroke-linejoin="round" d="M19 9l-7 7-7-7" />
                              {/if}
                            </svg>
                          {/if}
                        </span>
                      </th>
                    {/each}
                  </tr>
                </thead>
                <tbody class="divide-y divide-border">
                  {#each displayRows as row}
                    <tr class="hover:bg-muted/20 transition-colors">
                      {#each row as cell}
                        <td
                          class="px-3 py-2 font-mono text-foreground whitespace-nowrap max-w-xs truncate cursor-pointer"
                          on:click={() => copyCell(cell)}
                          title="Click to copy"
                        >
                          {#if cell === null}
                            <span class="text-muted-foreground italic opacity-60">NULL</span>
                          {:else}
                            {String(cell)}
                          {/if}
                        </td>
                      {/each}
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {:else if queryResult && queryResult.affected_rows >= 0}
            <div class="flex-1 flex items-center justify-center">
              <div class="text-center">
                <div class="w-12 h-12 rounded-2xl bg-emerald-500/10 flex items-center justify-center mx-auto mb-3">
                  <svg class="w-6 h-6 text-emerald-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
                  </svg>
                </div>
                <p class="text-sm font-medium text-foreground">Query executed</p>
                <p class="text-xs text-muted-foreground mt-1">
                  {queryResult.affected_rows} row(s) affected in {queryResult.duration_ms}ms
                </p>
              </div>
            </div>
          {/if}
        </div>

      <!-- ── Schema / ERD tab ──────────────────────────────────────────────── -->
      {:else if rightTab === 'schema'}
        <div class="flex-1 overflow-auto p-5">
          {#if tables.length === 0}
            <div class="flex items-center justify-center h-32">
              <p class="text-sm text-muted-foreground">Select a database to view its schema.</p>
            </div>
          {:else}
            <div class="flex items-center justify-between mb-4">
              <p class="text-xs text-muted-foreground">
                Schema for <strong class="text-foreground font-mono">{selectedDb.db_name}</strong>
                · {tables.length} tables
              </p>
              <div class="flex items-center gap-3 text-xs text-muted-foreground">
                <span class="flex items-center gap-1">
                  <span class="inline-block w-3 h-3 rounded bg-amber-500/20 border border-amber-500/30"></span>
                  PK — primary key
                </span>
                <span class="flex items-center gap-1">
                  <span class="inline-block w-3 h-3 rounded bg-blue-500/20 border border-blue-500/30"></span>
                  FK — foreign key
                </span>
              </div>
            </div>

            <div class="flex flex-wrap gap-4">
              {#each tables as table}
                <div class="rounded-xl border border-border bg-card min-w-[190px] overflow-hidden shadow-sm
                             hover:border-primary/30 transition-colors">
                  <!-- Table header -->
                  <div class="px-3 py-2.5 bg-primary/8 border-b border-border flex items-center justify-between gap-2">
                    <span class="text-xs font-semibold font-mono text-primary truncate">{table.name}</span>
                    <span class="text-[10px] text-muted-foreground shrink-0">{table.row_count.toLocaleString()} rows</span>
                  </div>
                  <!-- Columns -->
                  <div class="px-3 py-2.5 space-y-1.5">
                    {#if selectedTable?.name === table.name && browseResult}
                      {#each browseResult.columns as col}
                        <div class="flex items-center gap-1.5">
                          <span class="text-[11px] font-mono text-foreground">{col}</span>
                          {#if col === 'id'}
                            <span class="text-[9px] bg-amber-500/10 text-amber-500 dark:text-amber-400 px-1 py-0.5 rounded font-medium">PK</span>
                          {:else if col.endsWith('_id')}
                            <span class="text-[9px] bg-blue-500/10 text-blue-500 dark:text-blue-400 px-1 py-0.5 rounded font-medium">
                              FK → {getRefTable(col)}
                            </span>
                          {/if}
                        </div>
                      {/each}
                    {:else}
                      <span class="text-[11px] text-muted-foreground italic">Browse table to see columns</span>
                    {/if}
                  </div>
                  <!-- Size -->
                  <div class="px-3 pb-2.5">
                    <span class="text-[10px] text-muted-foreground">{formatBytes(table.size_bytes)}</span>
                  </div>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
</div>

<!-- ── DB Credentials Modal ──────────────────────────────────────────────── -->
{#if showCredsModal && credsDb}
  <!-- svelte-ignore a11y-click-events-have-key-events -->
  <!-- svelte-ignore a11y-no-static-element-interactions -->
  <div class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-center justify-center p-4"
       on:click|self={() => showCredsModal = false}>
    <div class="bg-card border border-border rounded-2xl p-6 w-full max-w-lg shadow-2xl">
      <!-- Header -->
      <div class="flex items-start justify-between mb-5">
        <div>
          <h3 class="text-base font-semibold text-foreground">Database Credentials</h3>
          <p class="text-xs text-muted-foreground font-mono mt-0.5">{credsDb.db_name}</p>
        </div>
        <button
          type="button"
          on:click={() => showCredsModal = false}
          class="w-8 h-8 rounded-lg flex items-center justify-center text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M6 18L18 6M6 6l12 12"/>
          </svg>
        </button>
      </div>

      <!-- Credentials grid -->
      <div class="space-y-3 mb-5">
        {#each [
          ['Database name', credsDb.db_name, 'db_name'],
          ['Username', credsDb.db_user, 'db_user'],
          ['Host', credsDb.db_host, 'db_host'],
          ['Port', String(credsDb.db_port), 'db_port'],
        ] as [label, value, key]}
          <div class="flex items-center gap-3">
            <span class="text-xs text-muted-foreground w-28 shrink-0">{label}</span>
            <div class="flex-1 flex items-center gap-2 min-w-0">
              <code class="flex-1 text-xs font-mono text-foreground bg-muted/40 rounded-lg px-3 py-2
                           border border-border truncate">{value}</code>
              <button
                type="button"
                on:click={() => copyToClipboard(value, key)}
                class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0 text-muted-foreground
                       hover:bg-muted hover:text-foreground transition-colors border border-border"
                title="Copy"
              >
                {#if credsCopied === key}
                  <svg class="w-3.5 h-3.5 text-emerald-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/>
                  </svg>
                {:else}
                  <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                    <rect x="9" y="9" width="13" height="13" rx="2"/>
                    <path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/>
                  </svg>
                {/if}
              </button>
            </div>
          </div>
        {/each}

        <!-- Password row with reveal toggle -->
        <div class="flex items-center gap-3">
          <span class="text-xs text-muted-foreground w-28 shrink-0">Password</span>
          <div class="flex-1 flex items-center gap-2 min-w-0">
            <code class="flex-1 text-xs font-mono text-foreground bg-muted/40 rounded-lg px-3 py-2
                         border border-border truncate">{credsPassword}</code>
            <button
              type="button"
              on:click={revealPassword}
              class="w-8 h-8 rounded-lg flex items-center justify-center shrink-0 text-muted-foreground
                     hover:bg-muted hover:text-foreground transition-colors border border-border"
              title={credsPasswordVisible ? 'Hide' : 'Reveal'}
            >
              {#if credsPasswordLoading}
                <div class="w-3 h-3 rounded-full border border-border border-t-primary animate-spin"></div>
              {:else if credsPasswordVisible}
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>
                  <line x1="2" y1="2" x2="22" y2="22"/>
                </svg>
              {:else}
                <svg class="w-3.5 h-3.5" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2">
                  <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7z"/><circle cx="12" cy="12" r="3"/>
                </svg>
              {/if}
            </button>
          </div>
        </div>
      </div>

      <!-- Connection strings -->
      <div class="mb-5 space-y-3">
        <!-- URL-style connection string -->
        <div>
          <div class="flex items-center justify-between mb-1.5">
            <span class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">Connection URL</span>
            <button
              type="button"
              on:click={() => copyToClipboard(buildUrlConnectionString(credsDb!), 'url')}
              class="h-6 px-2 rounded text-[10px] text-muted-foreground hover:bg-muted transition-colors
                     inline-flex items-center gap-1 border border-border"
            >
              {credsCopied === 'url' ? '✓ Copied' : 'Copy'}
            </button>
          </div>
          <code class="block text-xs font-mono text-foreground bg-muted/40 rounded-xl px-3 py-2.5
                       border border-border break-all leading-relaxed">
            {buildUrlConnectionString(credsDb)}
          </code>
        </div>

        <!-- CLI connection string -->
        <div>
          <div class="flex items-center justify-between mb-1.5">
            <span class="text-xs font-semibold text-muted-foreground uppercase tracking-wider">CLI Connection String</span>
            <button
              type="button"
              on:click={() => copyToClipboard(buildConnectionString(credsDb!), 'cli')}
              class="h-6 px-2 rounded text-[10px] text-muted-foreground hover:bg-muted transition-colors
                     inline-flex items-center gap-1 border border-border"
            >
              {credsCopied === 'cli' ? '✓ Copied' : 'Copy'}
            </button>
          </div>
          <code class="block text-xs font-mono text-foreground bg-muted/40 rounded-xl px-3 py-2.5
                       border border-border break-all leading-relaxed">
            {buildConnectionString(credsDb)}
          </code>
        </div>
      </div>

      <!-- Actions -->
      <div class="flex items-center gap-2.5">
        <button
          type="button"
          on:click={() => { selectDb(credsDb!); showCredsModal = false; }}
          class="h-9 px-4 rounded-lg bg-primary text-primary-foreground text-sm font-medium
                 hover:bg-primary/90 inline-flex items-center gap-2 transition-colors"
        >
          <svg class="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
            <path stroke-linecap="round" stroke-linejoin="round" d="M8 9l-4 3 4 3M16 9l4 3-4 3M12 5l-2 14"/>
          </svg>
          Open Query Editor
        </button>
        <button
          type="button"
          on:click={() => showCredsModal = false}
          class="h-9 px-4 rounded-lg border border-border text-sm text-muted-foreground
                 hover:bg-muted hover:text-foreground transition-colors ml-auto"
        >
          Close
        </button>
      </div>
    </div>
  </div>
{/if}

<!-- ── Toast ─────────────────────────────────────────────────────────────── -->
{#if toastMessage}
  <div
    class="fixed bottom-4 right-4 z-50 bg-card border border-border rounded-xl px-4 py-3 text-sm
           shadow-lg flex items-center gap-2.5 animate-in fade-in slide-in-from-bottom-2"
    role="status"
    aria-live="polite"
  >
    {#if toastType === 'success'}
      <div class="w-5 h-5 rounded-full bg-emerald-500/20 flex items-center justify-center shrink-0">
        <svg class="w-3 h-3 text-emerald-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7"/>
        </svg>
      </div>
    {:else}
      <div class="w-5 h-5 rounded-full bg-destructive/20 flex items-center justify-center shrink-0">
        <svg class="w-3 h-3 text-destructive" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="2.5">
          <path stroke-linecap="round" stroke-linejoin="round" d="M12 8v4m0 4h.01"/>
        </svg>
      </div>
    {/if}
    <span class="text-foreground">{toastMessage}</span>
  </div>
{/if}
