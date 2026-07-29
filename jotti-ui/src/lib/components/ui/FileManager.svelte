<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { t } from '$lib/i18n';
  import { api } from '$api/client';

  // ── Props ──────────────────────────────────────────────────────────────────
  export let siteId: string;
  export let initialPath: string = '/';
  export let embedded: boolean = false;
  export let unixUser: string = '';

  // ── Types ──────────────────────────────────────────────────────────────────
  type FileEntry = {
    name: string; path: string; size: number; is_dir: boolean;
    modified_at: string; permissions: string; owner: string;
  };
  type ViewMode = 'list' | 'grid';
  type SortKey = 'name' | 'size' | 'modified_at';

  // ── State ──────────────────────────────────────────────────────────────────
  let fmEl: HTMLElement;
  let fmFocused = false;
  let currentPath = initialPath || '/';
  let entries: FileEntry[] = [];
  let loading = true;
  let dirLoading = false;
  let selected: Set<string> = new Set();
  let pendingDelete: { path: string; isDir: boolean } | null = null;
  let pendingDeleteBatch = false;
  let lastSelectedPath: string | null = null;
  let viewMode: ViewMode = 'list';
  let clipboard: { paths: string[]; op: 'copy' | 'cut' } | null = null;
  let showHidden = false;
  let searchQuery = '';
  let sortKey: SortKey = 'name';
  let sortAsc = true;

  // History
  let navHistory: string[] = [];
  let histIdx = -1;

  // Path bar
  let editingPath = false;
  let pathInput = '';

  // Drag & drop / upload
  let dragOver = false;
  let uploading = false;
  let uploadProgress: { name: string; pct: number }[] = [];
  let fileInputEl: HTMLInputElement;

  // Toast
  let toast = '';
  let toastType: 'success' | 'error' = 'success';
  let toastTimer: ReturnType<typeof setTimeout>;

  // Context menu
  let ctxVisible = false;
  let ctxX = 0, ctxY = 0;
  let ctxEntry: FileEntry | null = null;

  // Inline rename
  let inlineRenaming: string | null = null;
  let inlineRenameVal = '';
  let inlineRenameEl: HTMLInputElement;

  // Modals
  let showRename = false, renameTarget = '', renameValue = '';
  let showNewFolder = false, newFolderName = '';
  let showNewFile = false, newFileName = '';
  let showEdit = false, editFile = '', editContent = '', editSaving = false;
  let showChmod = false, chmodTarget = '', chmodValue = '0644';
  let showCompress = false, compressName = 'archive.tar.gz', compressFormat = 'tar.gz';
  let showHelp = false;
  let showImagePreview = false, previewSrc = '', previewName = '';

  // Background right-click (empty area)
  let bgCtxVisible = false;
  let bgCtxX = 0, bgCtxY = 0;

  // Properties modal
  let showProperties = false;
  let propertiesEntry: FileEntry | null = null;

  // ── Icon helpers ───────────────────────────────────────────────────────────
  const EXT_ICONS: Record<string, string> = {
    jpg:'image',jpeg:'image',png:'image',gif:'image',webp:'image',svg:'image',ico:'image',bmp:'image',
    mp4:'video',webm:'video',avi:'video',mkv:'video',mov:'video',
    mp3:'audio',wav:'audio',ogg:'audio',flac:'audio',aac:'audio',
    zip:'archive',tar:'archive',gz:'archive',tgz:'archive',bz2:'archive','7z':'archive',rar:'archive',xz:'archive',
    pdf:'pdf',
    js:'js',mjs:'js',cjs:'js',ts:'ts',tsx:'ts',jsx:'js',
    html:'html',htm:'html',css:'css',scss:'css',sass:'css',less:'css',
    php:'php',py:'python',rb:'ruby',rs:'rust',go:'go',java:'java',
    json:'json',yaml:'yaml',yml:'yaml',toml:'config',xml:'xml',
    sql:'database',sh:'script',bash:'script',zsh:'script',fish:'script',
    md:'markdown',txt:'text',log:'log',csv:'csv',
    env:'config',gitignore:'git',lock:'lock',
  };

  const ICON_COLORS: Record<string, string> = {
    dir:'#f59e0b', image:'#10b981', video:'#8b5cf6', audio:'#ec4899',
    archive:'#f97316', pdf:'#ef4444', js:'#eab308', ts:'#3b82f6',
    html:'#f97316', css:'#8b5cf6', php:'#6366f1', python:'#3b82f6',
    ruby:'#ef4444', rust:'#f97316', go:'#06b6d4', java:'#f97316',
    json:'#10b981', yaml:'#f59e0b', xml:'#f97316', database:'#06b6d4',
    script:'#6b7280', markdown:'#6b7280', text:'#9ca3af', log:'#9ca3af',
    csv:'#22c55e', config:'#f59e0b', git:'#f97316', lock:'#ef4444', file:'#6b7280',
  };

  function fileType(name: string, is_dir: boolean): string {
    if (is_dir) return 'dir';
    return EXT_ICONS[name.split('.').pop()?.toLowerCase() ?? ''] ?? 'file';
  }
  function ic(t: string): string { return ICON_COLORS[t] ?? '#6b7280'; }

  function iconSvg(icon: string, color: string, size = 20): string {
    const s = size;
    if (icon === 'dir') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="${color}"><path d="M2 6a2 2 0 012-2h4l2 2h8a2 2 0 012 2v9a2 2 0 01-2 2H4a2 2 0 01-2-2V6z"/></svg>`;
    if (icon === 'image') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24"><rect x="2" y="3" width="20" height="18" rx="2" fill="${color}" opacity=".15"/><circle cx="8" cy="9" r="2" fill="${color}"/><path d="M2 16l6-5 4 4 3-3 5 5H2" fill="${color}"/></svg>`;
    if (icon === 'video') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="1.5"><rect x="2" y="5" width="14" height="14" rx="2"/><path d="M16 10l6-4v12l-6-4V10z" fill="${color}"/></svg>`;
    if (icon === 'audio') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="1.5"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3" fill="${color}"/><circle cx="18" cy="16" r="3" fill="${color}"/></svg>`;
    if (icon === 'archive') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="1.5"><path d="M5 3h14a2 2 0 012 2v14a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2z"/><path d="M9 3v18M15 3v18M9 9h6M9 15h6"/></svg>`;
    if (icon === 'pdf') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="1.5"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8l-6-6z"/><path d="M14 2v6h6"/><text x="5" y="19" fill="${color}" font-size="7" font-weight="bold" font-family="sans-serif">PDF</text></svg>`;
    if (icon === 'js' || icon === 'ts') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24"><rect width="24" height="24" rx="4" fill="${color}" opacity=".15"/><text x="3" y="17" fill="${color}" font-size="9" font-weight="900" font-family="monospace">${icon.toUpperCase()}</text></svg>`;
    if (icon === 'php') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24"><rect width="24" height="24" rx="4" fill="${color}" opacity=".15"/><text x="1" y="17" fill="${color}" font-size="8" font-weight="900" font-family="monospace">PHP</text></svg>`;
    if (icon === 'html') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24"><rect width="24" height="24" rx="4" fill="${color}" opacity=".15"/><text x="-1" y="17" fill="${color}" font-size="7" font-weight="900" font-family="monospace">HTML</text></svg>`;
    if (icon === 'css') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24"><rect width="24" height="24" rx="4" fill="${color}" opacity=".15"/><text x="1" y="17" fill="${color}" font-size="8" font-weight="900" font-family="monospace">CSS</text></svg>`;
    if (icon === 'database') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="1.5"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14c0 1.66 4.03 3 9 3s9-1.34 9-3V5"/><path d="M3 12c0 1.66 4.03 3 9 3s9-1.34 9-3"/></svg>`;
    if (icon === 'script') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="1.5"><path d="M8 9l-4 3 4 3M16 9l4 3-4 3M12 5l-2 14"/></svg>`;
    if (icon === 'markdown') return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="1.5"><rect x="2" y="4" width="20" height="16" rx="2"/><path d="M7 15V9l3 3 3-3v6M17 15v-4M15 13h4"/></svg>`;
    return `<svg width="${s}" height="${s}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="1.5"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8l-6-6z"/><path d="M14 2v6h6"/></svg>`;
  }

  // ── Init ───────────────────────────────────────────────────────────────────
  onMount(async () => {
    await loadDir(initialPath || '/');
    document.addEventListener('click', onDocClick);
    document.addEventListener('keydown', onKeyDown);
  });

  onDestroy(() => {
    document.removeEventListener('click', onDocClick);
    document.removeEventListener('keydown', onKeyDown);
    clearTimeout(toastTimer);
    if (previewSrc.startsWith('blob:')) URL.revokeObjectURL(previewSrc);
  });

  // ── Navigation ─────────────────────────────────────────────────────────────
  async function loadDir(path: string, pushHist = true) {
    dirLoading = true;
    selected = new Set();
    searchQuery = '';
    try {
      const resp = await api.filemanager.list({ site_id: siteId, path });
      entries     = resp.entries || [];
      currentPath = resp.path   || path;
      if (pushHist) {
        navHistory = [...navHistory.slice(0, histIdx + 1), currentPath];
        histIdx = navHistory.length - 1;
      }
    } catch (e: any) {
      // If requested path doesn't exist and is not root, fall back to root
      if (path !== '/') {
        dirLoading = false;
        loading    = false;
        await loadDir('/', pushHist);
        return;
      }
      showToast(e.message || 'Failed to load directory', 'error');
    } finally {
      dirLoading = false;
      loading    = false;
    }
  }

  async function navigateEntry(entry: FileEntry) {
    if (entry.is_dir) {
      await loadDir(entry.path);
    } else if (isImage(entry.name)) {
      await openImagePreview(entry);
    } else if (isEditable(entry.name)) {
      await openEditor(entry);
    }
  }

  async function navUp() {
    if (currentPath === '/') return;
    const parent = currentPath.split('/').slice(0, -1).join('/') || '/';
    await loadDir(parent);
  }

  async function navBack() {
    if (histIdx <= 0) return;
    histIdx--;
    await loadDir(navHistory[histIdx], false);
  }

  async function navFwd() {
    if (histIdx >= navHistory.length - 1) return;
    histIdx++;
    await loadDir(navHistory[histIdx], false);
  }

  function crumb(idx: number) {
    const parts = currentPath.split('/').filter(Boolean);
    loadDir(idx < 0 ? '/' : '/' + parts.slice(0, idx + 1).join('/'));
  }

  function startPathEdit() {
    pathInput = currentPath;
    editingPath = true;
  }
  async function commitPathEdit() {
    editingPath = false;
    if (pathInput && pathInput !== currentPath) await loadDir(pathInput);
  }

  // ── Sort / filter ──────────────────────────────────────────────────────────
  $: displayEntries = entries
    .filter(e => {
      if (!showHidden && e.name.startsWith('.')) return false;
      if (searchQuery) return e.name.toLowerCase().includes(searchQuery.toLowerCase());
      return true;
    })
    .sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      let cmp = 0;
      if (sortKey === 'name') cmp = a.name.localeCompare(b.name);
      else if (sortKey === 'size') cmp = a.size - b.size;
      else cmp = a.modified_at.localeCompare(b.modified_at);
      return sortAsc ? cmp : -cmp;
    });

  function toggleSort(key: SortKey) {
    if (sortKey === key) sortAsc = !sortAsc;
    else { sortKey = key; sortAsc = true; }
  }

  // ── Selection ──────────────────────────────────────────────────────────────
  function handleClick(entry: FileEntry, e: MouseEvent) {
    if (inlineRenaming) { commitInlineRename(); return; }
    if (e.ctrlKey || e.metaKey) {
      const s = new Set(selected);
      s.has(entry.path) ? s.delete(entry.path) : s.add(entry.path);
      selected = s; lastSelectedPath = entry.path;
    } else if (e.shiftKey && lastSelectedPath) {
      const paths = displayEntries.map(e => e.path);
      const from = paths.indexOf(lastSelectedPath), to = paths.indexOf(entry.path);
      if (from !== -1 && to !== -1) {
        const [lo, hi] = from < to ? [from, to] : [to, from];
        selected = new Set(paths.slice(lo, hi + 1));
      }
    } else {
      if (selected.size > 0) { selected = new Set(); return; }
      navigateEntry(entry);
    }
  }

  function chkToggle(path: string, e: Event) {
    e.stopPropagation();
    const s = new Set(selected);
    s.has(path) ? s.delete(path) : s.add(path);
    selected = s; lastSelectedPath = path;
  }

  function selectAll() { selected = new Set(displayEntries.map(e => e.path)); }
  function clearSel()  { selected = new Set(); }

  // ── Inline rename ──────────────────────────────────────────────────────────
  function startInlineRename(entry: FileEntry) {
    inlineRenaming = entry.path;
    inlineRenameVal = entry.name;
    setTimeout(() => { inlineRenameEl?.focus(); inlineRenameEl?.select(); }, 20);
  }

  async function commitInlineRename() {
    const old = inlineRenaming;
    inlineRenaming = null;
    if (!old || !inlineRenameVal.trim() || inlineRenameVal === old.split('/').pop()) return;
    const dir = currentPath === '/' ? '' : currentPath;
    const newPath = (dir + '/' + inlineRenameVal).replace('//', '/');
    try {
      await api.filemanager.rename({ site_id: siteId, path: old, new_path: newPath });
      showToast('Renamed', 'success'); await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Rename failed', 'error'); }
  }

  // ── Context menu ──────────────────────────────────────────────────────────
  function openCtx(e: MouseEvent, entry: FileEntry) {
    e.preventDefault(); e.stopPropagation();
    bgCtxVisible = false;
    ctxEntry = entry; ctxX = e.clientX; ctxY = e.clientY; ctxVisible = true;
  }

  function openBgCtx(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (target.closest('.fm-row') || target.closest('.gc') || target.closest('.fm-table thead')) return;
    e.preventDefault();
    ctxVisible = false;
    bgCtxX = e.clientX; bgCtxY = e.clientY; bgCtxVisible = true;
  }

  function onDocClick() { ctxVisible = false; bgCtxVisible = false; }

  async function copyPathToClipboard(path: string) {
    try {
      await navigator.clipboard.writeText(path);
      showToast('Path copied', 'success');
    } catch { showToast('Copy failed', 'error'); }
  }

  async function duplicateFile(entry: FileEntry) {
    if (entry.is_dir) return;
    const dot = entry.name.lastIndexOf('.');
    const base = dot > 0 ? entry.name.slice(0, dot) : entry.name;
    const ext  = dot > 0 ? entry.name.slice(dot) : '';
    const dir  = currentPath === '/' ? '' : currentPath;
    const dest = (dir + '/' + base + '-copy' + ext).replace('//', '/');
    try {
      const token = localStorage.getItem('orbit_access_token') ?? '';
      const readRes = await fetch(
        `/api/v1/files/read?site_id=${siteId}&path=${encodeURIComponent(entry.path)}`,
        { headers: { Authorization: `Bearer ${token}` } }
      );
      if (!readRes.ok) throw new Error('Cannot read file');
      const data = await readRes.json() as { content?: string };
      await api.filemanager.write({ site_id: siteId, path: dest, content: data.content ?? '' });
      showToast(`Duplicated as ${base}-copy${ext}`, 'success');
      api.invalidateCache('files');
      await loadDir(currentPath);
    } catch (e: unknown) { showToast((e as { message?: string }).message || 'Duplicate failed', 'error'); }
  }

  async function ctxAction(action: string, entry?: FileEntry) {
    const t = entry ?? ctxEntry;
    ctxVisible = false;
    bgCtxVisible = false;
    if (!t && !['paste', 'new-folder', 'new-file', 'upload', 'refresh', 'select-all'].includes(action)) return;
    switch (action) {
      case 'open':       await navigateEntry(t!); break;
      case 'edit':       if (!t!.is_dir) await openEditor(t!); break;
      case 'preview':    await openImagePreview(t!); break;
      case 'rename':     startInlineRename(t!); break;
      case 'chmod':      chmodTarget = t!.path; chmodValue = t!.permissions || '0644'; showChmod = true; break;
      case 'download':   window.open(api.filemanager.downloadUrl(siteId, t!.path), '_blank'); break;
      case 'delete':     await deleteOne(t!.path, t!.is_dir); break;
      case 'copy':       clipboard = { paths: [t!.path], op: 'copy' }; showToast('Copied', 'success'); break;
      case 'cut':        clipboard = { paths: [t!.path], op: 'cut' };  showToast('Cut', 'success'); break;
      case 'extract':    await extractFile(t!); break;
      case 'copy-path':  await copyPathToClipboard(t!.path); break;
      case 'duplicate':  await duplicateFile(t!); break;
      case 'properties': propertiesEntry = t!; showProperties = true; break;
      // Background actions
      case 'paste':      await pasteClipboard(); break;
      case 'new-folder': showNewFolder = true; break;
      case 'new-file':   showNewFile = true; break;
      case 'upload':     fileInputEl?.click(); break;
      case 'refresh':    api.invalidateCache('files'); await loadDir(currentPath); break;
      case 'select-all': selectAll(); break;
    }
  }

  // ── Image preview ──────────────────────────────────────────────────────────
  async function openImagePreview(entry: FileEntry) {
    try {
      const token = localStorage.getItem('orbit_access_token') ?? '';
      const res = await fetch(
        `/api/v1/files/read?site_id=${siteId}&path=${encodeURIComponent(entry.path)}`,
        { headers: { Authorization: `Bearer ${token}` } }
      );
      if (previewSrc.startsWith('blob:')) URL.revokeObjectURL(previewSrc);
      previewSrc = res.ok ? URL.createObjectURL(await res.blob())
                          : api.filemanager.downloadUrl(siteId, entry.path);
    } catch {
      previewSrc = api.filemanager.downloadUrl(siteId, entry.path);
    }
    previewName = entry.name;
    showImagePreview = true;
  }

  // ── File operations ────────────────────────────────────────────────────────
  async function deleteOne(path: string, isDir: boolean) {
    pendingDelete = { path, isDir };
  }

  async function confirmDeleteOne() {
    if (!pendingDelete) return;
    const { path, isDir } = pendingDelete;
    pendingDelete = null;
    try {
      await api.filemanager.delete({ site_id: siteId, path, recursive: isDir });
      showToast('Deleted', 'success'); await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Delete failed', 'error'); }
  }

  async function deleteSelected() {
    if (!selected.size) return;
    pendingDeleteBatch = true;
  }

  async function confirmDeleteSelected() {
    pendingDeleteBatch = false;
    const count = selected.size;
    try {
      for (const path of Array.from(selected)) {
        const entry = entries.find(e => e.path === path);
        await api.filemanager.delete({ site_id: siteId, path, recursive: entry?.is_dir ?? false });
      }
      showToast(`${count} item(s) deleted`, 'success');
      selected = new Set(); await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Delete failed', 'error'); }
  }

  async function doRename() {
    if (!renameValue.trim()) return;
    const dir = currentPath === '/' ? '' : currentPath;
    const newPath = (dir + '/' + renameValue).replace('//', '/');
    try {
      await api.filemanager.rename({ site_id: siteId, path: renameTarget, new_path: newPath });
      showToast('Renamed', 'success'); showRename = false; await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Rename failed', 'error'); }
  }

  async function doMkdir() {
    if (!newFolderName.trim()) return;
    const path = (currentPath + '/' + newFolderName).replace('//', '/');
    try {
      await api.filemanager.mkdir({ site_id: siteId, path });
      showToast('Folder created', 'success'); showNewFolder = false; newFolderName = '';
      await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Create failed', 'error'); }
  }

  async function doNewFile() {
    if (!newFileName.trim()) return;
    const path = (currentPath + '/' + newFileName).replace('//', '/');
    try {
      await api.filemanager.write({ site_id: siteId, path, content: '' });
      showToast('File created', 'success'); showNewFile = false; newFileName = '';
      await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Create failed', 'error'); }
  }

  async function openEditor(entry: FileEntry) {
    try {
      const token = localStorage.getItem('orbit_access_token') ?? '';
      const res = await fetch(
        `/api/v1/files/read?site_id=${siteId}&path=${encodeURIComponent(entry.path)}`,
        { headers: { Authorization: `Bearer ${token}` } }
      );
      if (!res.ok) throw new Error('Cannot read file (binary or too large)');
      const data = await res.json();
      editContent = data.content ?? '';
      editFile    = entry.path;
      showEdit    = true;
    } catch (e: any) { showToast(e.message, 'error'); }
  }

  async function saveEdit() {
    editSaving = true;
    try {
      await api.filemanager.write({ site_id: siteId, path: editFile, content: editContent });
      showToast('Saved', 'success'); showEdit = false;
    } catch (e: any) { showToast(e.message || 'Save failed', 'error'); }
    finally { editSaving = false; }
  }

  async function doChmod() {
    const token = localStorage.getItem('orbit_access_token') ?? '';
    try {
      const res = await fetch('/api/v1/files/chmod', {
        method: 'POST',
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ site_id: siteId, path: chmodTarget, mode: chmodValue })
      });
      if (!res.ok) throw new Error('chmod failed');
      showToast('Permissions updated', 'success'); showChmod = false;
      await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'chmod failed', 'error'); }
  }

  async function doCompress() {
    const token = localStorage.getItem('orbit_access_token') ?? '';
    try {
      const res = await fetch('/api/v1/files/compress', {
        method: 'POST',
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({
          site_id: siteId, paths: Array.from(selected),
          output: (currentPath + '/' + compressName).replace('//', '/'),
          format: compressFormat
        })
      });
      if (!res.ok) throw new Error('Compress failed');
      showToast('Archive created', 'success'); showCompress = false;
      await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Compress failed', 'error'); }
  }

  async function extractFile(entry: FileEntry) {
    const token = localStorage.getItem('orbit_access_token') ?? '';
    try {
      const res = await fetch('/api/v1/files/extract', {
        method: 'POST',
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
        body: JSON.stringify({ site_id: siteId, archive: entry.path })
      });
      if (!res.ok) throw new Error('Extract failed');
      showToast('Extracted', 'success'); await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Extract failed', 'error'); }
  }

  async function pasteClipboard() {
    if (!clipboard) return;
    try {
      for (const src of clipboard.paths) {
        const dst = (currentPath + '/' + src.split('/').pop()).replace('//', '/');
        await api.filemanager.rename({ site_id: siteId, path: src, new_path: dst });
      }
      if (clipboard.op === 'cut') clipboard = null;
      showToast('Paste complete', 'success'); await loadDir(currentPath);
    } catch (e: any) { showToast(e.message || 'Paste failed', 'error'); }
  }

  // ── Upload with XHR progress ───────────────────────────────────────────────
  async function handleFiles(fileList: FileList | null) {
    if (!fileList || !fileList.length) return;
    uploading = true;
    uploadProgress = Array.from(fileList).map(f => ({ name: f.name, pct: 0 }));
    try {
      await Promise.all(Array.from(fileList).map((f, i) => uploadOne(f, i)));
      showToast(`${fileList.length} file(s) uploaded`, 'success');
      api.invalidateCache('files');
      await loadDir(currentPath);
    } catch { showToast('Upload failed', 'error'); }
    finally { uploading = false; dragOver = false; uploadProgress = []; }
  }

  function uploadOne(file: File, idx: number): Promise<void> {
    return new Promise((resolve, reject) => {
      const token = localStorage.getItem('orbit_access_token') ?? '';
      const fd = new FormData();
      fd.append('file', file); fd.append('site_id', siteId); fd.append('path', currentPath);
      const xhr = new XMLHttpRequest();
      xhr.upload.onprogress = (e) => {
        if (e.lengthComputable)
          uploadProgress = uploadProgress.map((p, i2) =>
            i2 === idx ? { ...p, pct: Math.round(e.loaded / e.total * 100) } : p);
      };
      xhr.onload = () => xhr.status < 300 ? resolve() : reject(new Error('Upload failed'));
      xhr.onerror = () => reject(new Error('Network error'));
      xhr.open('POST', '/api/v1/files/upload');
      xhr.setRequestHeader('Authorization', `Bearer ${token}`);
      xhr.send(fd);
    });
  }

  function onDrop(e: DragEvent) {
    e.preventDefault(); dragOver = false;
    handleFiles(e.dataTransfer?.files ?? null);
  }

  // ── Keyboard shortcuts ─────────────────────────────────────────────────────
  function onKeyDown(e: KeyboardEvent) {
    if (embedded && fmEl && !fmEl.contains(document.activeElement) && document.activeElement !== document.body) return;
    if (showEdit || showRename || showNewFolder || showNewFile || showChmod || editingPath || inlineRenaming) return;
    if (e.key === '?') { showHelp = true; return; }
    if (e.key === 'Escape') { clearSel(); ctxVisible = false; bgCtxVisible = false; showHelp = false; showImagePreview = false; showProperties = false; return; }
    if (e.key === 'F5') { e.preventDefault(); loadDir(currentPath); return; }
    if (e.key === 'Backspace' && !e.ctrlKey && !e.altKey) { e.preventDefault(); navUp(); return; }
    if (e.key === 'Delete' && selected.size > 0) { e.preventDefault(); deleteSelected(); return; }
    if (e.key === 'F2' && selected.size === 1) {
      const entry = entries.find(en => en.path === Array.from(selected)[0]);
      if (entry) startInlineRename(entry);
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      if (e.key === 'a') { e.preventDefault(); selectAll(); return; }
      if (e.key === 'c' && selected.size > 0) { clipboard = { paths: Array.from(selected), op: 'copy' }; showToast(`${selected.size} copied`, 'success'); return; }
      if (e.key === 'x' && selected.size > 0) { clipboard = { paths: Array.from(selected), op: 'cut' };  showToast(`${selected.size} cut`, 'success'); return; }
      if (e.key === 'v' && clipboard) { pasteClipboard(); return; }
    }
  }

  // ── Toast ──────────────────────────────────────────────────────────────────
  function showToast(msg: string, type: 'success' | 'error') {
    toast = msg; toastType = type;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toast = '', 3500);
  }

  // ── Helpers ────────────────────────────────────────────────────────────────
  function fmtSize(n: number): string {
    if (!n) return '—';
    if (n < 1024) return `${n} B`;
    if (n < 1048576) return `${(n/1024).toFixed(1)} KB`;
    if (n < 1073741824) return `${(n/1048576).toFixed(1)} MB`;
    return `${(n/1073741824).toFixed(2)} GB`;
  }

  function fmtDate(s: string): string {
    try {
      const d = new Date(s);
      return d.toLocaleDateString(undefined, { month:'short', day:'numeric', year:'numeric' })
        + ' ' + d.toLocaleTimeString(undefined, { hour:'2-digit', minute:'2-digit' });
    } catch { return s; }
  }

  function isArchive(n: string) { return /\.(zip|tar\.gz|tgz|tar\.bz2|tar\.xz)$/i.test(n); }
  function isEditable(n: string) { return /\.(txt|md|html?|css|scss|sass|less|js|ts|jsx|tsx|json|ya?ml|toml|xml|php|py|rb|sh|bash|env|conf|cfg|ini|log|csv|sql)$/i.test(n); }
  function isImage(n: string)    { return /\.(jpe?g|png|gif|webp|svg|bmp|ico)$/i.test(n); }

  $: pathParts  = currentPath.split('/').filter(Boolean);
  $: totalSize  = entries.filter(e => !e.is_dir).reduce((s, e) => s + e.size, 0);
  $: selSize    = entries.filter(e => selected.has(e.path) && !e.is_dir).reduce((s, e) => s + e.size, 0);
  $: canBack    = histIdx > 0;
  $: canFwd     = histIdx < navHistory.length - 1;

  // ── chmod visual ──────────────────────────────────────────────────────────
  type RwxBit = { r: boolean; w: boolean; x: boolean };
  let chmodBits: RwxBit[] = [{r:false,w:false,x:false},{r:false,w:false,x:false},{r:false,w:false,x:false}];

  function octToRwx(oct: string): RwxBit[] {
    const n = parseInt(oct.replace(/^0+/, '') || '0', 8);
    return [
      { r: !!(n & 0o400), w: !!(n & 0o200), x: !!(n & 0o100) },
      { r: !!(n & 0o040), w: !!(n & 0o020), x: !!(n & 0o010) },
      { r: !!(n & 0o004), w: !!(n & 0o002), x: !!(n & 0o001) },
    ];
  }
  function rwxToOct(bits: RwxBit[]): string {
    let n = 0;
    if (bits[0].r) n |= 0o400; if (bits[0].w) n |= 0o200; if (bits[0].x) n |= 0o100;
    if (bits[1].r) n |= 0o040; if (bits[1].w) n |= 0o020; if (bits[1].x) n |= 0o010;
    if (bits[2].r) n |= 0o004; if (bits[2].w) n |= 0o002; if (bits[2].x) n |= 0o001;
    return '0' + n.toString(8).padStart(3, '0');
  }

  $: if (showChmod) chmodBits = octToRwx(chmodValue);
  function syncBitsToOctal() { chmodValue = rwxToOct(chmodBits); }
  function syncOctalToBits() { if (/^0?[0-7]{3,4}$/.test(chmodValue)) chmodBits = octToRwx(chmodValue); }
</script>

<!-- ── Toast ─────────────────────────────────────────────────────────────── -->
{#if toast}
  <div class="fm-toast {toastType === 'success' ? 'toast-ok' : 'toast-err'}">{toast}</div>
{/if}

<!-- ── Shell ─────────────────────────────────────────────────────────────── -->
<div
  class="fm-shell"
  class:fm-embedded={embedded}
  bind:this={fmEl}
  tabindex="-1"
  on:focusin={() => fmFocused = true}
  on:focusout={e => { if (!fmEl?.contains(e.relatedTarget as Node)) fmFocused = false; }}
>
  <!-- ══ Sidebar ══════════════════════════════════════════════════════════ -->
  <aside class="fm-sidebar">
    <div class="fm-sb-head">
      <div class="fm-sb-icon">
        <svg fill="#f59e0b" viewBox="0 0 20 20" width="14" height="14">
          <path d="M2 6a2 2 0 012-2h4l2 2h6a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z"/>
        </svg>
      </div>
      <span>File Manager</span>
    </div>

    {#if unixUser}
      <div class="fm-sb-root">
        <svg fill="none" viewBox="0 0 16 16" width="12" height="12" stroke="currentColor" stroke-width="1.5">
          <path d="M8 1L1 7h2v7h4v-4h2v4h4V7h2L8 1z" stroke-linejoin="round"/>
        </svg>
        <span>/home/{unixUser}</span>
      </div>
    {/if}

    <nav class="fm-tree">
      <!-- Root -->
      <button class="fm-ti {currentPath === '/' ? 'fti-on' : ''}" on:click={() => loadDir('/')}>
        <svg viewBox="0 0 20 20" width="13" height="13" fill="#6b7280">
          <path d="M10.707 2.293a1 1 0 00-1.414 0l-7 7a1 1 0 001.414 1.414L4 10.414V17a1 1 0 001 1h2a1 1 0 001-1v-2a1 1 0 011-1h2a1 1 0 011 1v2a1 1 0 001 1h2a1 1 0 001-1v-6.586l.293.293a1 1 0 001.414-1.414l-7-7z"/>
        </svg>
        <span>/ root</span>
      </button>

      <!-- Breadcrumb tree -->
      {#each pathParts as part, i}
        <button
          class="fm-ti {i === pathParts.length - 1 ? 'fti-on' : ''}"
          style="padding-left:{(i + 2) * 12}px"
          on:click={() => crumb(i)}>
          <svg viewBox="0 0 16 16" width="12" height="12" fill="#f59e0b">
            <path d="M1 5a1 1 0 011-1h3l1 1h6a1 1 0 011 1v6a1 1 0 01-1 1H2a1 1 0 01-1-1V5z"/>
          </svg>
          <span>{part}</span>
        </button>
        {#if i === pathParts.length - 1}
          {#each displayEntries.filter(e => e.is_dir) as dir}
            <button
              class="fm-ti"
              style="padding-left:{(i + 3) * 12}px"
              on:click={() => loadDir(dir.path)}>
              <svg viewBox="0 0 16 16" width="12" height="12" fill="#f59e0b">
                <path d="M1 5a1 1 0 011-1h3l1 1h6a1 1 0 011 1v6a1 1 0 01-1 1H2a1 1 0 01-1-1V5z"/>
              </svg>
              <span class="text-xs">{dir.name}</span>
            </button>
          {/each}
        {/if}
      {/each}
    </nav>

    <!-- Sidebar quick stats -->
    <div class="fm-sb-foot">
      <div class="fm-sb-stat">
        <span>{displayEntries.length} items</span>
        <span>{fmtSize(totalSize)}</span>
      </div>
    </div>
  </aside>

  <!-- ══ Main ═════════════════════════════════════════════════════════════ -->
  <main class="fm-main">

    <!-- ── Address bar ───────────────────────────────────────────────────── -->
    <div class="fm-addrbar">
      <!-- Nav buttons -->
      <div class="fm-nav-btns">
        <button class="fnb fnb-home" on:click={() => loadDir('/')} title="Home">
          <svg fill="none" viewBox="0 0 20 20" width="14" height="14" stroke="currentColor" stroke-width="2.2">
            <path stroke-linecap="round" stroke-linejoin="round" d="M3 9.5L10 3l7 6.5V17a1 1 0 01-1 1h-4v-4H8v4H4a1 1 0 01-1-1V9.5z"/>
          </svg>
        </button>
        <div class="fnb-divider"></div>
        <button class="fnb" disabled={!canBack} on:click={navBack} title="Back">
          <svg fill="none" viewBox="0 0 20 20" width="14" height="14" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M13 15l-5-5 5-5"/>
          </svg>
        </button>
        <button class="fnb" disabled={!canFwd} on:click={navFwd} title="Forward">
          <svg fill="none" viewBox="0 0 20 20" width="14" height="14" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M7 5l5 5-5 5"/>
          </svg>
        </button>
        <button class="fnb" disabled={currentPath === '/'} on:click={navUp} title="Up (Backspace)">
          <svg fill="none" viewBox="0 0 20 20" width="14" height="14" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M10 17V3M4 9l6-6 6 6"/>
          </svg>
        </button>
        <button class="fnb fnb-refresh {dirLoading ? 'fnb-spinning' : ''}" on:click={() => { api.invalidateCache('files'); loadDir(currentPath); }} title="Refresh (F5)">
          <svg fill="none" viewBox="0 0 20 20" width="14" height="14" stroke="currentColor" stroke-width="2.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 4v4h4M16 16v-4h-4M4.08 12A8 8 0 1016 8"/>
          </svg>
        </button>
      </div>

      <!-- Path breadcrumb / input -->
      <div class="fm-path-wrap">
        {#if editingPath}
          <input class="fm-path-edit" bind:value={pathInput}
            on:blur={commitPathEdit}
            on:keydown={e => { if (e.key === 'Enter') commitPathEdit(); if (e.key === 'Escape') editingPath = false; }}
            autofocus />
        {:else}
          <button class="fm-path-crumb" on:click={startPathEdit} title="Click to edit path">
            <span class="pc-home" on:click|stopPropagation={() => loadDir('/')}>
              <svg fill="none" viewBox="0 0 16 16" width="12" height="12" stroke="currentColor" stroke-width="1.8">
                <path stroke-linejoin="round" d="M8 1L1 7h2v7h4v-4h2v4h4V7h2L8 1z"/>
              </svg>
            </span>
            {#each pathParts as part, i}
              <span class="pc-sep">/</span>
              {#if i === pathParts.length - 1}
                <span class="pc-cur">{part}</span>
              {:else}
                <span class="pc-lnk" on:click|stopPropagation={() => crumb(i)}>{part}</span>
              {/if}
            {/each}
            {#if !pathParts.length}<span class="pc-cur">root</span>{/if}
          </button>
        {/if}
      </div>

      <!-- Search -->
      <div class="fm-search">
        <svg fill="none" viewBox="0 0 16 16" width="12" height="12" stroke="currentColor" stroke-width="2">
          <circle cx="7" cy="7" r="5"/><path d="M12 12l2.5 2.5"/>
        </svg>
        <input class="fm-search-inp" type="text" placeholder="Filter…" bind:value={searchQuery} />
        {#if searchQuery}
          <button class="fm-search-x" on:click={() => searchQuery = ''}>×</button>
        {/if}
      </div>
    </div>

    <!-- ── Inline delete confirm banners ────────────────────────────────── -->
    {#if pendingDelete}
      <div class="flex items-center gap-2 px-3 py-2 bg-destructive/10 border-b border-destructive/20 text-xs text-destructive">
        <span>Delete "<strong>{pendingDelete.path.split('/').pop()}</strong>"? This cannot be undone.</span>
        <button class="px-2 py-0.5 rounded bg-destructive text-white hover:bg-destructive/90" on:click={confirmDeleteOne}>Delete</button>
        <button class="px-2 py-0.5 rounded bg-muted text-foreground" on:click={() => pendingDelete = null}>Cancel</button>
      </div>
    {/if}
    {#if pendingDeleteBatch}
      <div class="flex items-center gap-2 px-3 py-2 bg-destructive/10 border-b border-destructive/20 text-xs text-destructive">
        <span>Delete <strong>{selected.size}</strong> item(s)? This cannot be undone.</span>
        <button class="px-2 py-0.5 rounded bg-destructive text-white hover:bg-destructive/90" on:click={confirmDeleteSelected}>Delete all</button>
        <button class="px-2 py-0.5 rounded bg-muted text-foreground" on:click={() => pendingDeleteBatch = false}>Cancel</button>
      </div>
    {/if}

    <!-- ── Toolbar ────────────────────────────────────────────────────────── -->
    <div class="fm-toolbar">
      <!-- Left: actions -->
      <div class="fm-tb-left">
        {#if selected.size > 0}
          <div class="fm-sel-pill">{selected.size} selected</div>
          <button class="tb-btn" on:click={() => { clipboard={paths:Array.from(selected),op:'copy'}; showToast(`${selected.size} copied`,'success'); }}>
            <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="1.8"><rect x="5" y="5" width="9" height="9" rx="1"/><path d="M10 5V3a1 1 0 00-1-1H3a1 1 0 00-1 1v7a1 1 0 001 1h2"/></svg>
            Copy
          </button>
          <button class="tb-btn" on:click={() => { clipboard={paths:Array.from(selected),op:'cut'}; showToast(`${selected.size} cut`,'success'); }}>
            <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="1.8"><circle cx="5" cy="12" r="2"/><circle cx="11" cy="12" r="2"/><path d="M5 10L9 5M11 10L7 5M9 3l-4 4M7 3l4 4"/></svg>
            Cut
          </button>
          <button class="tb-btn" on:click={() => showCompress = true}>
            <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="1.8"><rect x="2" y="2" width="12" height="12" rx="1"/><path d="M6 2v12M10 2v12M6 6h4M6 10h4"/></svg>
            Compress
          </button>
          <button class="tb-btn tb-danger" on:click={deleteSelected}>
            <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M2 4h12M5 4V3a1 1 0 011-1h4a1 1 0 011 1v1M13 4l-.867 9.142A1 1 0 0111.138 14H4.862a1 1 0 01-.995-.858L3 4"/></svg>
            Delete
          </button>
          <div class="tb-sep"></div>
        {/if}

        {#if clipboard}
          <button class="tb-btn tb-accent" on:click={pasteClipboard}>
            <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M5 2H3a1 1 0 00-1 1v11a1 1 0 001 1h10a1 1 0 001-1V3a1 1 0 00-1-1h-2"/><path d="M6 1h4a1 1 0 010 2H6a1 1 0 010-2z"/></svg>
            Paste ({clipboard.paths.length})
          </button>
          <button class="tb-btn" on:click={() => clipboard = null}>✕</button>
          <div class="tb-sep"></div>
        {/if}

        <button class="tb-btn tb-primary" on:click={() => fileInputEl?.click()} disabled={uploading}>
          <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M8 1v9M4 5l4-4 4 4M2 13v1a1 1 0 001 1h10a1 1 0 001-1v-1"/></svg>
          {uploading ? 'Uploading…' : 'Upload'}
        </button>
        <input type="file" multiple class="sr-only" bind:this={fileInputEl}
          on:change={e => handleFiles((e.target as HTMLInputElement).files)} />

        <button class="tb-btn" on:click={() => showNewFolder = true}>
          <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M8 9h4M10 7v4M1 4a1 1 0 011-1h3l1 1h7a1 1 0 011 1v8a1 1 0 01-1 1H2a1 1 0 01-1-1V4z"/></svg>
          New Folder
        </button>

        <button class="tb-btn" on:click={() => showNewFile = true}>
          <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M9 1H3a1 1 0 00-1 1v12a1 1 0 001 1h10a1 1 0 001-1V6l-5-5z"/><path d="M9 1v5h5M8 9v4M6 11h4"/></svg>
          New File
        </button>
      </div>

      <!-- Right: view options -->
      <div class="fm-tb-right">
        <button class="tb-icon {showHidden ? 'tb-on' : ''}" on:click={() => showHidden = !showHidden}
          title={showHidden ? 'Hide dotfiles' : 'Show dotfiles'}>
          <svg fill="none" viewBox="0 0 16 16" width="14" height="14" stroke="currentColor" stroke-width="1.8">
            {#if showHidden}
              <circle cx="8" cy="8" r="3"/><path d="M1 8s2.5-5 7-5 7 5 7 5-2.5 5-7 5-7-5-7-5z"/>
            {:else}
              <path d="M2 2l12 12M6.5 6.6A3 3 0 0111.4 9.5M4.3 4.3C2.8 5.4 1.7 6.7 1 8c1.3 2.5 4.1 5 7 5 1.2 0 2.4-.4 3.4-1M8 5c.2 0 .4 0 .6.1"/><path d="M1 8s2.5-5 7-5c.9 0 1.7.1 2.5.4"/>
            {/if}
          </svg>
        </button>
        <div class="tb-view-seg">
          <button class="tvs-btn {viewMode === 'list' ? 'tvs-on' : ''}" on:click={() => viewMode = 'list'} title="List view">
            <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="2">
              <path d="M2 4h12M2 8h12M2 12h12"/>
            </svg>
          </button>
          <button class="tvs-btn {viewMode === 'grid' ? 'tvs-on' : ''}" on:click={() => viewMode = 'grid'} title="Grid view">
            <svg fill="none" viewBox="0 0 16 16" width="13" height="13" stroke="currentColor" stroke-width="2">
              <rect x="1" y="1" width="6" height="6" rx="1"/><rect x="9" y="1" width="6" height="6" rx="1"/>
              <rect x="1" y="9" width="6" height="6" rx="1"/><rect x="9" y="9" width="6" height="6" rx="1"/>
            </svg>
          </button>
        </div>
        <button class="tb-icon" on:click={() => showHelp = true} title="Keyboard shortcuts">
          <svg fill="none" viewBox="0 0 16 16" width="14" height="14" stroke="currentColor" stroke-width="1.8">
            <circle cx="8" cy="8" r="7"/><path d="M8 11v1M8 5a2 2 0 011.732 3C9.246 8.55 8 9 8 10"/>
          </svg>
        </button>
      </div>
    </div>

    <!-- Upload progress bar -->
    {#if uploading && uploadProgress.length}
      <div class="fm-upbar">
        {#each uploadProgress as p}
          <div class="up-row">
            <span class="up-name">{p.name}</span>
            <div class="up-track"><div class="up-fill" style="width:{p.pct}%"></div></div>
            <span class="up-pct">{p.pct}%</span>
          </div>
        {/each}
      </div>
    {/if}

    <!-- ── File area ──────────────────────────────────────────────────────── -->
    <div
      class="fm-area {dragOver ? 'drag-active' : ''}"
      on:dragover|preventDefault={() => dragOver = true}
      on:dragleave={() => dragOver = false}
      on:drop={onDrop}
      on:contextmenu={openBgCtx}
    >
      {#if dirLoading || loading}
        <div class="fm-loading">
          <div class="fm-spin"></div>
          <span class="text-xs text-[var(--fm-muted)] mt-2">Loading…</span>
        </div>

      {:else if !displayEntries.length && !searchQuery}
        <div class="fm-empty">
          <div class="fm-empty-icon">
            <svg fill="none" viewBox="0 0 48 48" width="40" height="40" stroke="currentColor" stroke-width="1.5">
              <path d="M6 14a4 4 0 014-4h8l4 4h16a4 4 0 014 4v18a4 4 0 01-4 4H10a4 4 0 01-4-4V14z" stroke-linecap="round" stroke-linejoin="round"/>
            </svg>
          </div>
          <p class="font-medium">Empty directory</p>
          <p class="text-sm opacity-60 mt-1">Drop files here or click Upload</p>
        </div>

      {:else if !displayEntries.length && searchQuery}
        <div class="fm-empty">
          <p class="font-medium">No results for "{searchQuery}"</p>
          <button class="mt-2 text-sm" style="color:var(--fm-accent)" on:click={() => searchQuery = ''}>Clear filter</button>
        </div>

      {:else if viewMode === 'list'}
        <!-- List view -->
        <div class="fm-list-wrap">
          <table class="fm-table">
            <thead>
              <tr class="fm-thead-row">
                <th class="tc-chk">
                  <input type="checkbox"
                    checked={selected.size > 0 && selected.size === displayEntries.length}
                    indeterminate={selected.size > 0 && selected.size < displayEntries.length}
                    on:change={e => (e.target as HTMLInputElement).checked ? selectAll() : clearSel()} />
                </th>
                <th class="tc-name sortable" on:click={() => toggleSort('name')}>
                  Name
                  {#if sortKey === 'name'}<span class="sort-arrow">{sortAsc ? '↑' : '↓'}</span>{/if}
                </th>
                <th class="tc-size sortable" on:click={() => toggleSort('size')}>
                  Size
                  {#if sortKey === 'size'}<span class="sort-arrow">{sortAsc ? '↑' : '↓'}</span>{/if}
                </th>
                <th class="tc-date sortable" on:click={() => toggleSort('modified_at')}>
                  Modified
                  {#if sortKey === 'modified_at'}<span class="sort-arrow">{sortAsc ? '↑' : '↓'}</span>{/if}
                </th>
                <th class="tc-perm">Perms</th>
                <th class="tc-act"></th>
              </tr>
            </thead>
            <tbody>
              {#if currentPath !== '/'}
                <tr class="fm-row" on:click={navUp}>
                  <td class="tc-chk"></td>
                  <td class="tc-name" colspan="4">
                    <div class="row-inner">
                      <div class="row-icon-wrap">
                        <svg fill="none" viewBox="0 0 18 18" width="16" height="16" stroke="currentColor" stroke-width="2">
                          <path stroke-linecap="round" stroke-linejoin="round" d="M9 14l-6-5m0 0l6-5m-6 5h12"/>
                        </svg>
                      </div>
                      <span class="row-name-text muted">.. parent directory</span>
                    </div>
                  </td>
                  <td class="tc-act"></td>
                </tr>
              {/if}

              {#each displayEntries as entry (entry.path)}
                {@const icon = fileType(entry.name, entry.is_dir)}
                {@const color = ic(icon)}
                <tr
                  class="fm-row {selected.has(entry.path) ? 'fm-row-sel' : ''}"
                  on:click={e => handleClick(entry, e)}
                  on:dblclick={() => navigateEntry(entry)}
                  on:contextmenu={e => openCtx(e, entry)}
                >
                  <td class="tc-chk" on:click|stopPropagation={e => chkToggle(entry.path, e)}>
                    <input type="checkbox" checked={selected.has(entry.path)}
                      on:click|stopPropagation
                      on:change={e => chkToggle(entry.path, e)} />
                  </td>
                  <td class="tc-name">
                    <div class="row-inner">
                      <div class="row-icon-wrap">
                        {@html iconSvg(icon, color, 17)}
                      </div>
                      {#if inlineRenaming === entry.path}
                        <input
                          class="inline-rename-inp"
                          bind:value={inlineRenameVal}
                          bind:this={inlineRenameEl}
                          on:keydown={e => {
                            if (e.key === 'Enter') { e.stopPropagation(); commitInlineRename(); }
                            if (e.key === 'Escape') { e.stopPropagation(); inlineRenaming = null; }
                          }}
                          on:blur={commitInlineRename}
                          on:click|stopPropagation
                        />
                      {:else}
                        <span class="row-name-text {entry.is_dir ? 'is-dir' : ''}">{entry.name}</span>
                      {/if}
                    </div>
                  </td>
                  <td class="tc-size">{entry.is_dir ? '—' : fmtSize(entry.size)}</td>
                  <td class="tc-date">{fmtDate(entry.modified_at)}</td>
                  <td class="tc-perm">{entry.permissions}</td>
                  <td class="tc-act" on:click|stopPropagation>
                    <div class="row-acts">
                      {#if isImage(entry.name)}
                        <button class="ra-btn" title="Preview" on:click={() => openImagePreview(entry)}>
                          <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><circle cx="7" cy="7" r="2.5"/><path d="M1 7s2-4.5 6-4.5S13 7 13 7s-2 4.5-6 4.5S1 7 1 7z"/></svg>
                        </button>
                      {/if}
                      {#if !entry.is_dir && isEditable(entry.name)}
                        <button class="ra-btn" title="Edit" on:click={() => ctxAction('edit', entry)}>
                          <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M9 2l3 3L5 12H2V9L9 2z"/><path d="M7.5 3.5l3 3"/></svg>
                        </button>
                      {/if}
                      {#if !entry.is_dir}
                        <button class="ra-btn" title="Download" on:click={() => ctxAction('download', entry)}>
                          <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M7 1v7M4 5l3 4 3-4M2 11v1a1 1 0 001 1h8a1 1 0 001-1v-1"/></svg>
                        </button>
                      {/if}
                      <button class="ra-btn" title="Rename (F2)" on:click|stopPropagation={() => startInlineRename(entry)}>
                        <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M1 10v2h3l7-7-3-3-7 7z"/><path d="M9 3l2 2"/></svg>
                      </button>
                      <button class="ra-btn ra-del" title="Delete" on:click={() => ctxAction('delete', entry)}>
                        <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M2 3h10M5 3V2h4v1M11 3l-.714 8.5A1 1 0 019.29 12H4.71a1 1 0 01-.996-.5L3 3M6 6v4M8 6v4"/></svg>
                      </button>
                    </div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>

      {:else}
        <!-- Grid view -->
        <div class="fm-grid">
          {#if currentPath !== '/'}
            <button class="gc" on:click={navUp}>
              <div class="gc-icon">
                <svg fill="none" viewBox="0 0 24 24" width="32" height="32" stroke="#9ca3af" stroke-width="1.5">
                  <path stroke-linecap="round" stroke-linejoin="round" d="M11 17l-5-5m0 0l5-5m-5 5h12"/>
                </svg>
              </div>
              <span class="gc-name muted">..</span>
            </button>
          {/if}
          {#each displayEntries as entry (entry.path)}
            {@const icon = fileType(entry.name, entry.is_dir)}
            {@const color = ic(icon)}
            <div class="gc {selected.has(entry.path) ? 'gc-sel' : ''}"
              on:click={e => handleClick(entry, e)}
              on:dblclick={() => navigateEntry(entry)}
              on:contextmenu={e => openCtx(e, entry)}>
              <div class="gc-chk" on:click|stopPropagation={e => chkToggle(entry.path, e)}>
                <input type="checkbox" checked={selected.has(entry.path)} on:change={e => chkToggle(entry.path, e)} />
              </div>
              <div class="gc-icon">{@html iconSvg(icon, color, 34)}</div>
              <span class="gc-name">{entry.name}</span>
              {#if !entry.is_dir}<span class="gc-size">{fmtSize(entry.size)}</span>{/if}
            </div>
          {/each}
        </div>
      {/if}

      {#if dragOver}
        <div class="fm-drop-cover">
          <svg fill="none" viewBox="0 0 24 24" width="40" height="40" stroke="white" stroke-width="1.5">
            <path stroke-linecap="round" stroke-linejoin="round" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-8l-4-4m0 0L8 8m4-4v12"/>
          </svg>
          <p>Drop to upload</p>
        </div>
      {/if}
    </div>

    <!-- ── Status bar ─────────────────────────────────────────────────────── -->
    <div class="fm-statusbar">
      <div class="fm-sb-l">
        <span>{displayEntries.length} item{displayEntries.length !== 1 ? 's' : ''}</span>
        {#if searchQuery}<span class="sb-muted">of {entries.length}</span>{/if}
        {#if selected.size > 0}
          <span class="sb-sel">{selected.size} selected{selSize ? ` · ${fmtSize(selSize)}` : ''}</span>
        {/if}
        {#if clipboard}
          <span class="sb-clip">{clipboard.paths.length} in clipboard ({clipboard.op})</span>
        {/if}
      </div>
      <div class="fm-sb-r">
        {#if totalSize}<span class="sb-muted">{fmtSize(totalSize)} total</span>{/if}
      </div>
    </div>

  </main>
</div>

<!-- ══ Context menu ══════════════════════════════════════════════════════ -->
{#if ctxVisible && ctxEntry}
  <div class="fm-ctx" style="left:{ctxX}px;top:{ctxY}px" on:click|stopPropagation>
    {#if ctxEntry.is_dir}
      <button class="ctx-it" on:click={() => ctxAction('open')}>
        <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M1 4a1 1 0 011-1h3l1 1h6a1 1 0 011 1v6a1 1 0 01-1 1H2a1 1 0 01-1-1V4z"/></svg>
        <span>Open</span>
      </button>
    {:else}
      {#if isImage(ctxEntry.name)}
        <button class="ctx-it" on:click={() => ctxAction('preview')}>
          <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><circle cx="7" cy="7" r="2.5"/><path d="M1 7s2-4.5 6-4.5S13 7 13 7s-2 4.5-6 4.5S1 7 1 7z"/></svg>
          <span>Preview</span>
        </button>
      {/if}
      {#if isEditable(ctxEntry.name)}
        <button class="ctx-it" on:click={() => ctxAction('edit')}>
          <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M9 2l3 3L5 12H2V9L9 2z"/></svg>
          <span>Edit</span><kbd class="ctx-kbd">Enter</kbd>
        </button>
      {/if}
      <button class="ctx-it" on:click={() => ctxAction('download')}>
        <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M7 1v7M4 5l3 4 3-4M2 11v2h10v-2"/></svg>
        <span>Download</span>
      </button>
      {#if isArchive(ctxEntry.name)}
        <button class="ctx-it" on:click={() => ctxAction('extract')}>
          <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><rect x="1" y="1" width="12" height="12" rx="1"/><path d="M5 1v12M9 1v12M5 5h4M5 9h4"/></svg>
          <span>Extract Here</span>
        </button>
      {/if}
      <button class="ctx-it" on:click={() => ctxAction('duplicate')}>
        <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><rect x="1" y="4" width="9" height="9" rx="1"/><path d="M4 4V3a1 1 0 011-1h7a1 1 0 011 1v7a1 1 0 01-1 1h-1"/></svg>
        <span>Duplicate</span>
      </button>
    {/if}

    <div class="ctx-sep"></div>

    <button class="ctx-it" on:click={() => ctxAction('copy')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><rect x="5" y="5" width="8" height="8" rx="1"/><path d="M9 5V3a1 1 0 00-1-1H3a1 1 0 00-1 1v6a1 1 0 001 1h2"/></svg>
      <span>Copy</span><kbd class="ctx-kbd">Ctrl+C</kbd>
    </button>
    <button class="ctx-it" on:click={() => ctxAction('cut')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><circle cx="4" cy="11" r="2"/><circle cx="10" cy="11" r="2"/><path d="M4 9L7.5 4M10 9L6.5 4M7.5 2l-3 3M6.5 2l3 3"/></svg>
      <span>Cut</span><kbd class="ctx-kbd">Ctrl+X</kbd>
    </button>
    {#if clipboard}
      <button class="ctx-it" on:click={() => ctxAction('paste')}>
        <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M5 2H3a1 1 0 00-1 1v9a1 1 0 001 1h8a1 1 0 001-1V3a1 1 0 00-1-1h-2"/><path d="M5 1h4a1 1 0 010 2H5a1 1 0 010-2z"/></svg>
        <span>Paste</span><kbd class="ctx-kbd">Ctrl+V</kbd>
      </button>
    {/if}

    <div class="ctx-sep"></div>

    <button class="ctx-it" on:click={() => ctxAction('rename')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M1 10v2h3l7-7-3-3-7 7z"/><path d="M9 3l2 2"/></svg>
      <span>Rename</span><kbd class="ctx-kbd">F2</kbd>
    </button>
    <button class="ctx-it" on:click={() => ctxAction('chmod')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><rect x="3" y="6" width="8" height="7" rx="1"/><path d="M5 6V4a3 3 0 016 0v2"/></svg>
      <span>Permissions</span>
    </button>
    <button class="ctx-it" on:click={() => ctxAction('copy-path')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M9 3h2a1 1 0 011 1v7a1 1 0 01-1 1H3a1 1 0 01-1-1V4a1 1 0 011-1h2"/><rect x="4" y="1" width="6" height="3" rx="1"/></svg>
      <span>Copy Path</span>
    </button>
    <button class="ctx-it" on:click={() => ctxAction('properties')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><circle cx="7" cy="7" r="6"/><path d="M7 6.5v4"/><circle cx="7" cy="4.5" r=".6" fill="currentColor" stroke="none"/></svg>
      <span>Properties</span>
    </button>

    <div class="ctx-sep"></div>

    <button class="ctx-it ctx-danger" on:click={() => ctxAction('delete')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M2 3h10M5 3V2h4v1M11 3l-.714 8.5A1 1 0 019.29 12H4.71a1 1 0 01-.996-.5L3 3"/></svg>
      <span>Delete</span><kbd class="ctx-kbd ctx-kbd-danger">Del</kbd>
    </button>
  </div>
{/if}

<!-- ══ Background context menu ════════════════════════════════════════════ -->
{#if bgCtxVisible}
  <div class="fm-ctx" style="left:{bgCtxX}px;top:{bgCtxY}px" on:click|stopPropagation>
    <button class="ctx-it" on:click={() => ctxAction('new-folder')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M1 4a1 1 0 011-1h3l1 1h6a1 1 0 011 1v6a1 1 0 01-1 1H2a1 1 0 01-1-1V4zM7 7v3M5.5 8.5h3"/></svg>
      <span>New Folder</span>
    </button>
    <button class="ctx-it" on:click={() => ctxAction('new-file')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M8 1H3a1 1 0 00-1 1v10a1 1 0 001 1h8a1 1 0 001-1V6l-4-5z"/><path d="M8 1v5h4M7 8v3M5.5 9.5h3"/></svg>
      <span>New File</span>
    </button>
    <button class="ctx-it" on:click={() => ctxAction('upload')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M7 1v7M4 4l3-3 3 3M2 10v2a1 1 0 001 1h8a1 1 0 001-1v-2"/></svg>
      <span>Upload Files</span>
    </button>
    <div class="ctx-sep"></div>
    {#if clipboard}
      <button class="ctx-it" on:click={() => ctxAction('paste')}>
        <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M5 2H3a1 1 0 00-1 1v9a1 1 0 001 1h8a1 1 0 001-1V3a1 1 0 00-1-1h-2"/><path d="M5 1h4a1 1 0 010 2H5a1 1 0 010-2z"/></svg>
        <span>Paste ({clipboard.paths.length})</span><kbd class="ctx-kbd">Ctrl+V</kbd>
      </button>
      <div class="ctx-sep"></div>
    {/if}
    <button class="ctx-it" on:click={() => ctxAction('select-all')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><rect x="1" y="1" width="12" height="12" rx="1.5" stroke-dasharray="3 1.5"/></svg>
      <span>Select All</span><kbd class="ctx-kbd">Ctrl+A</kbd>
    </button>
    <button class="ctx-it" on:click={() => ctxAction('refresh')}>
      <svg fill="none" viewBox="0 0 14 14" width="13" height="13" stroke="currentColor" stroke-width="1.8"><path d="M2 4v3h3M12 10v-3h-3"/><path d="M2.5 7A5 5 0 0012 8.5M11.5 7A5 5 0 002 5.5"/></svg>
      <span>Refresh</span><kbd class="ctx-kbd">F5</kbd>
    </button>
  </div>
{/if}

<!-- ══ Modals ════════════════════════════════════════════════════════════ -->

{#if showRename}
  <div class="modal-overlay" on:click|self={() => showRename = false}>
    <div class="modal-box">
      <h3 class="modal-title" style="display:flex;align-items:center;gap:6px;" ><svg class="modal-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>Rename</h3>
      <input class="modal-input" bind:value={renameValue} autofocus
        on:keydown={e => e.key === 'Enter' && doRename()} />
      <div class="modal-actions">
        <button class="mb-primary" on:click={doRename}>Rename</button>
        <button class="mb-ghost" on:click={() => showRename = false}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

{#if showNewFolder}
  <div class="modal-overlay" on:click|self={() => showNewFolder = false}>
    <div class="modal-box">
      <h3 class="modal-title" style="display:flex;align-items:center;gap:6px;" ><svg class="modal-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>New Folder</h3>
      <input class="modal-input" placeholder="folder-name" bind:value={newFolderName} autofocus
        on:keydown={e => e.key === 'Enter' && doMkdir()} />
      <div class="modal-actions">
        <button class="mb-primary" on:click={doMkdir}>Create</button>
        <button class="mb-ghost" on:click={() => { showNewFolder = false; newFolderName = ''; }}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

{#if showNewFile}
  <div class="modal-overlay" on:click|self={() => showNewFile = false}>
    <div class="modal-box">
      <h3 class="modal-title" style="display:flex;align-items:center;gap:6px;" ><svg class="modal-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>New File</h3>
      <input class="modal-input" placeholder="filename.php" bind:value={newFileName} autofocus
        on:keydown={e => e.key === 'Enter' && doNewFile()} />
      <div class="modal-actions">
        <button class="mb-primary" on:click={doNewFile}>Create</button>
        <button class="mb-ghost" on:click={() => { showNewFile = false; newFileName = ''; }}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

{#if showChmod}
  <div class="modal-overlay" on:click|self={() => showChmod = false}>
    <div class="modal-box" style="max-width:400px">
      <h3 class="modal-title" style="display:flex;align-items:center;gap:6px;" ><svg class="modal-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>File Permissions</h3>
      <p class="modal-sub">{chmodTarget.split('/').pop()}</p>

      <!-- Visual rwx grid -->
      <div class="chmod-wrap">
        <div class="chmod-grid">
          <div></div>
          <div class="cg-hdr">Read</div>
          <div class="cg-hdr">Write</div>
          <div class="cg-hdr">Execute</div>
          {#each ['Owner','Group','Others'] as label, i}
            <div class="cg-lbl">{label}</div>
            {#each ['r','w','x'] as _, j}
              <div class="cg-cell">
                <input type="checkbox"
                  checked={j === 0 ? chmodBits[i].r : j === 1 ? chmodBits[i].w : chmodBits[i].x}
                  on:change={e => {
                    const v = (e.target as HTMLInputElement).checked;
                    if (j === 0) chmodBits[i] = {...chmodBits[i], r: v};
                    else if (j === 1) chmodBits[i] = {...chmodBits[i], w: v};
                    else chmodBits[i] = {...chmodBits[i], x: v};
                    chmodBits = [...chmodBits];
                    syncBitsToOctal();
                  }} />
              </div>
            {/each}
          {/each}
        </div>

        <div class="chmod-row mt-3">
          <span class="cg-lbl">Octal</span>
          <input class="modal-input font-mono" style="max-width:80px;flex:none" bind:value={chmodValue}
            on:input={syncOctalToBits} placeholder="0644" />
          <div class="flex gap-1 flex-wrap">
            {#each [['0644','Files'],['0755','Dirs'],['0777','Full'],['0600','Private']] as [m, l]}
              <button class="chmod-pre" on:click={() => { chmodValue = m; syncOctalToBits(); }}
                title={l}>{m}</button>
            {/each}
          </div>
        </div>
      </div>

      <div class="modal-actions mt-4">
        <button class="mb-primary" on:click={doChmod}>Apply</button>
        <button class="mb-ghost" on:click={() => showChmod = false}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

{#if showCompress}
  <div class="modal-overlay" on:click|self={() => showCompress = false}>
    <div class="modal-box">
      <h3 class="modal-title" style="display:flex;align-items:center;gap:6px;" ><svg class="modal-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>Compress {selected.size} item(s)</h3>
      <label class="modal-label">Archive name</label>
      <input class="modal-input mb-3" bind:value={compressName} />
      <label class="modal-label">Format</label>
      <select class="modal-input mb-3" bind:value={compressFormat}>
        <option value="tar.gz">tar.gz</option>
        <option value="zip">zip</option>
      </select>
      <div class="modal-actions">
        <button class="mb-primary" on:click={doCompress}>Compress</button>
        <button class="mb-ghost" on:click={() => showCompress = false}>Cancel</button>
      </div>
    </div>
  </div>
{/if}

{#if showEdit}
  <div class="modal-overlay" on:click|self={() => showEdit = false}>
    <div class="modal-editor">
      <div class="med-header">
        <div class="med-title">
          <svg fill="none" viewBox="0 0 14 14" width="14" height="14" stroke="var(--fm-accent)" stroke-width="1.8">
            <path d="M9 2l3 3L5 12H2V9L9 2z"/>
          </svg>
          <span class="font-semibold text-sm">{editFile.split('/').pop()}</span>
          <span class="med-path">{editFile}</span>
        </div>
        <div class="flex gap-2">
          <button class="mb-primary text-xs" on:click={saveEdit} disabled={editSaving}>
            {editSaving ? 'Saving…' : '✓ Save'}
          </button>
          <button class="mb-ghost text-xs" on:click={() => showEdit = false}>Close</button>
        </div>
      </div>
      <div class="med-hint">Ctrl+S to save · Tab inserts spaces</div>
      <textarea
        class="med-area"
        bind:value={editContent}
        spellcheck="false"
        on:keydown={e => {
          if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); saveEdit(); }
          if (e.key === 'Tab') {
            e.preventDefault();
            const el = e.target as HTMLTextAreaElement;
            const s = el.selectionStart, end = el.selectionEnd;
            editContent = editContent.slice(0,s) + '  ' + editContent.slice(end);
            setTimeout(() => { el.selectionStart = el.selectionEnd = s + 2; }, 0);
          }
        }}
      ></textarea>
    </div>
  </div>
{/if}

{#if showImagePreview}
  <div class="modal-overlay" on:click|self={() => showImagePreview = false}>
    <div class="modal-preview">
      <div class="mp-header">
        <span class="font-medium text-sm">{previewName}</span>
        <div class="flex gap-2">
          <a href={previewSrc} download={previewName} class="mb-primary text-xs">↓ Download</a>
          <button class="mb-ghost text-xs" on:click={() => showImagePreview = false}>Close</button>
        </div>
      </div>
      <div class="mp-body">
        <img src={previewSrc} alt={previewName} class="mp-img" />
      </div>
    </div>
  </div>
{/if}

{#if showHelp}
  <div class="modal-overlay" on:click|self={() => showHelp = false}>
    <div class="modal-box" style="max-width:420px">
      <h3 class="modal-title" style="display:flex;align-items:center;gap:6px;" ><svg class="modal-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>Keyboard Shortcuts</h3>
      <div class="help-grid">
        {#each [
          ['Backspace','Navigate up'],['Delete','Delete selected'],
          ['F2','Rename selected'],['F5','Refresh'],['Escape','Clear selection'],
          ['Ctrl+A','Select all'],['Ctrl+C','Copy'],['Ctrl+X','Cut'],['Ctrl+V','Paste'],
          ['Ctrl+S','Save (in editor)'],['Tab','Insert spaces (editor)'],
          ['?','Show this help'],['Click','Navigate / select'],
          ['Ctrl+Click','Multi-select'],['Shift+Click','Range select'],
          ['Dbl-click','Open / edit'],
        ] as [k, d]}
          <kbd class="hk">{k}</kbd><span class="hd">{d}</span>
        {/each}
      </div>
      <div class="modal-actions mt-4">
        <button class="mb-primary" on:click={() => showHelp = false}>Got it</button>
      </div>
    </div>
  </div>
{/if}

{#if showProperties && propertiesEntry}
  <div class="modal-overlay" on:click|self={() => showProperties = false}>
    <div class="modal-box" style="max-width:420px">
      <div class="prop-header">
        <div class="prop-icon">{@html iconSvg(fileType(propertiesEntry.name, propertiesEntry.is_dir), ic(fileType(propertiesEntry.name, propertiesEntry.is_dir)), 24)}</div>
        <h3 class="modal-title" style="margin:0;display:flex;align-items:center;gap:6px;" ><svg class="modal-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="2.8" fill="currentColor"/><ellipse cx="12" cy="12" rx="8" ry="3.2" stroke="currentColor" stroke-width="1.5" fill="none" transform="rotate(-30 12 12)"/><circle cx="18.9" cy="8.0" r="1.4" fill="currentColor"/></svg>{propertiesEntry.name}</h3>
      </div>
      <div class="prop-grid">
        <span class="prop-lbl">Type</span>
        <span class="prop-val">{propertiesEntry.is_dir ? 'Directory' : 'File'}</span>
        <span class="prop-lbl">Location</span>
        <span class="prop-val prop-mono">{propertiesEntry.path.split('/').slice(0, -1).join('/') || '/'}</span>
        {#if !propertiesEntry.is_dir}
          <span class="prop-lbl">Size</span>
          <span class="prop-val">{fmtSize(propertiesEntry.size)} <span class="prop-muted">({propertiesEntry.size.toLocaleString()} bytes)</span></span>
        {/if}
        <span class="prop-lbl">Modified</span>
        <span class="prop-val">{fmtDate(propertiesEntry.modified_at)}</span>
        <span class="prop-lbl">Permissions</span>
        <span class="prop-val prop-mono">{propertiesEntry.permissions}</span>
        {#if propertiesEntry.owner}
          <span class="prop-lbl">Owner</span>
          <span class="prop-val prop-mono">{propertiesEntry.owner}</span>
        {/if}
      </div>
      <div class="modal-actions mt-4">
        <button class="mb-primary" on:click={() => {
          chmodTarget = propertiesEntry!.path;
          chmodValue = propertiesEntry!.permissions || '0644';
          showProperties = false;
          showChmod = true;
        }}>Edit Permissions</button>
        <button class="mb-ghost" on:click={() => showProperties = false}>Close</button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ── CSS custom properties ── */
  /* Modals render as siblings of .fm-shell (not descendants), so the --fm-* vars
     must also be declared on the modal containers or var() resolves to transparent. */
  .fm-shell, .modal-overlay, .modal-editor, .modal-preview {
    --fm-bg: var(--bg-card, #ffffff);
    --fm-bg-side: var(--bg-sidebar, #f8fafc);
    --fm-bg-hover: var(--bg-hover, rgba(0,0,0,.04));
    --fm-bg-sel: rgba(99,102,241,.06);
    --fm-border: var(--border, #e2e8f0);
    --fm-text: var(--text, #0f172a);
    --fm-muted: var(--text-muted, #64748b);
    --fm-accent: var(--accent, #6366f1);
    --fm-danger: #ef4444;
  }
  :global(.dark) .fm-shell, :global(.dark) .modal-overlay, :global(.dark) .modal-editor, :global(.dark) .modal-preview {
    --fm-bg: #1e2128;
    --fm-bg-side: #161920;
    --fm-bg-hover: rgba(255,255,255,.04);
    --fm-bg-sel: rgba(99,102,241,.12);
    --fm-border: #2d3148;
    --fm-text: #f0f4ff;
    --fm-muted: #8892b0;
  }

  /* ── Shell layout ── */
  .fm-shell { display:flex; height:100%; min-height:520px; border:none; border-radius:0; overflow:hidden; background:var(--fm-bg); outline:none; font-family:inherit; }
  .fm-embedded { height:720px; min-height:480px; border:1px solid var(--fm-border); border-radius:10px; }

  /* ── Sidebar ── */
  .fm-sidebar { width:196px; flex-shrink:0; background:var(--fm-bg-side); border-right:1px solid var(--fm-border); display:flex; flex-direction:column; overflow:hidden; }
  .fm-sb-head { display:flex; align-items:center; gap:7px; padding:11px 12px 9px; border-bottom:1px solid var(--fm-border); font-size:11px; font-weight:600; color:var(--fm-muted); text-transform:uppercase; letter-spacing:.06em; }
  .fm-sb-icon { display:flex; align-items:center; justify-content:center; width:22px; height:22px; background:rgba(245,158,11,.12); border-radius:5px; flex-shrink:0; }
  .fm-sb-root { display:flex; align-items:center; gap:6px; padding:6px 12px; font-size:11px; color:var(--fm-muted); font-family:monospace; overflow:hidden; }
  .fm-sb-root span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .fm-tree { flex:1; overflow-y:auto; padding:4px 0; }
  .fm-ti { display:flex; align-items:center; gap:6px; width:100%; padding:5px 12px; text-align:left; font-size:12px; color:var(--fm-text); cursor:pointer; transition:background .1s; border:none; background:transparent; }
  .fm-ti:hover { background:var(--fm-bg-hover); }
  .fm-ti span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .fti-on { background:rgba(99,102,241,.09) !important; color:var(--fm-accent); font-weight:500; }
  .fm-sb-foot { border-top:1px solid var(--fm-border); padding:6px 12px; flex-shrink:0; }
  .fm-sb-stat { display:flex; justify-content:space-between; font-size:11px; color:var(--fm-muted); }

  /* ── Main ── */
  .fm-main { flex:1; display:flex; flex-direction:column; overflow:hidden; min-width:0; background:var(--fm-bg); }

  /* ── Address bar ── */
  .fm-addrbar { display:flex; align-items:center; gap:6px; padding:7px 10px; border-bottom:1px solid var(--fm-border); background:var(--fm-bg); flex-shrink:0; }
  .fm-nav-btns { display:flex; gap:1px; flex-shrink:0; }
  .fnb { display:flex; align-items:center; justify-content:center; width:26px; height:26px; border-radius:5px; color:var(--fm-muted); transition:background .1s,color .1s; border:none; background:transparent; cursor:pointer; }
  .fnb:hover:not(:disabled) { background:var(--fm-bg-hover); color:var(--fm-text); }
  .fnb:disabled { opacity:.3; cursor:not-allowed; }
  .fnb-home { color:var(--fm-accent); }
  .fnb-home:hover:not(:disabled) { background:rgba(99,102,241,.1); color:var(--fm-accent); }
  .fnb-divider { width:1px; height:18px; background:var(--fm-border); margin:0 3px; align-self:center; flex-shrink:0; }
  .fnb-refresh svg { transition:transform .3s; transform-box:fill-box; transform-origin:center; }
  .fnb-spinning svg { animation:spin .65s linear infinite; transform-box:fill-box; transform-origin:center; }
  .fm-path-wrap { flex:1; min-width:0; }
  .fm-path-crumb { display:flex; align-items:center; gap:2px; width:100%; padding:3px 8px; height:28px; border-radius:7px; border:1px solid var(--fm-border); background:var(--fm-bg-side); font-size:12px; cursor:text; overflow:hidden; }
  .fm-path-edit { width:100%; height:28px; padding:0 8px; border-radius:7px; border:1.5px solid var(--fm-accent); background:var(--fm-bg-side); font-size:12px; color:var(--fm-text); outline:none; }
  .pc-home { display:flex; align-items:center; color:var(--fm-muted); cursor:pointer; padding:0 2px; flex-shrink:0; }
  .pc-home:hover { color:var(--fm-accent); }
  .pc-sep { color:var(--fm-muted); padding:0 1px; flex-shrink:0; }
  .pc-lnk { color:var(--fm-accent); cursor:pointer; padding:0 2px; white-space:nowrap; }
  .pc-lnk:hover { text-decoration:underline; }
  .pc-cur { font-weight:500; color:var(--fm-text); padding:0 2px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
  .fm-search { display:flex; align-items:center; gap:5px; padding:0 8px; height:28px; border:1px solid var(--fm-border); border-radius:7px; background:var(--fm-bg); flex-shrink:0; width:150px; }
  .fm-search-inp { flex:1; font-size:12px; color:var(--fm-text); background:transparent; border:none; outline:none; min-width:0; }
  .fm-search-inp::placeholder { color:var(--fm-muted); }
  .fm-search-x { font-size:15px; color:var(--fm-muted); cursor:pointer; flex-shrink:0; background:none; border:none; padding:0; line-height:1; }
  .fm-search-x:hover { color:var(--fm-text); }

  /* ── Toolbar ── */
  .fm-toolbar { display:flex; align-items:center; justify-content:space-between; padding:6px 10px; border-bottom:1px solid var(--fm-border); background:var(--fm-bg); flex-wrap:wrap; gap:4px; flex-shrink:0; }
  .fm-tb-left { display:flex; align-items:center; gap:4px; flex-wrap:wrap; }
  .fm-tb-right { display:flex; align-items:center; gap:4px; flex-shrink:0; }
  .fm-sel-pill { font-size:11px; font-weight:600; color:var(--fm-accent); padding:2px 8px; background:rgba(99,102,241,.1); border-radius:20px; white-space:nowrap; }
  .tb-btn { display:flex; align-items:center; gap:4px; padding:4px 9px; border-radius:6px; border:1px solid var(--fm-border); font-size:12px; color:var(--fm-text); background:var(--fm-bg); cursor:pointer; transition:background .1s; white-space:nowrap; }
  .tb-btn:hover:not(:disabled) { background:var(--fm-bg-hover); }
  .tb-btn:disabled { opacity:.5; cursor:not-allowed; }
  .tb-primary { background:var(--fm-accent) !important; color:#fff !important; border-color:transparent !important; font-weight:500; }
  .tb-primary:hover:not(:disabled) { opacity:.9; background:var(--fm-accent) !important; }
  .tb-accent { background:rgba(99,102,241,.1) !important; color:var(--fm-accent) !important; border-color:rgba(99,102,241,.25) !important; }
  .tb-danger { color:var(--fm-danger) !important; }
  .tb-danger:hover { background:rgba(239,68,68,.12) !important; }
  .tb-on { background:rgba(99,102,241,.1) !important; color:var(--fm-accent) !important; border-color:rgba(99,102,241,.25) !important; }
  .tb-sep { width:1px; height:18px; background:var(--fm-border); margin:0 2px; flex-shrink:0; }
  .tb-icon { display:flex; align-items:center; justify-content:center; width:28px; height:28px; border-radius:6px; border:1px solid var(--fm-border); color:var(--fm-muted); background:var(--fm-bg); cursor:pointer; transition:background .1s; }
  .tb-icon:hover { background:var(--fm-bg-hover); color:var(--fm-text); }
  .tb-view-seg { display:flex; border:1px solid var(--fm-border); border-radius:6px; overflow:hidden; }
  .tvs-btn { display:flex; align-items:center; justify-content:center; width:28px; height:26px; background:transparent; color:var(--fm-muted); cursor:pointer; border:none; transition:background .1s; }
  .tvs-btn:hover { background:var(--fm-bg-hover); }
  .tvs-on { background:var(--fm-accent) !important; color:#fff !important; }

  /* Upload progress */
  .fm-upbar { padding:6px 12px; border-bottom:1px solid var(--fm-border); background:var(--fm-bg); flex-shrink:0; }
  .up-row { display:flex; align-items:center; gap:8px; font-size:11px; margin-bottom:3px; }
  .up-name { width:140px; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--fm-text); }
  .up-track { flex:1; height:3px; background:var(--fm-border); border-radius:2px; overflow:hidden; }
  .up-fill { height:100%; background:var(--fm-accent); border-radius:2px; transition:width .15s; }
  .up-pct { width:30px; text-align:right; color:var(--fm-muted); }

  /* ── File area ── */
  .fm-area { flex:1; overflow-y:auto; position:relative; background:var(--fm-bg); }
  .drag-active { outline:2px dashed var(--fm-accent); outline-offset:-4px; }
  .fm-drop-cover { position:absolute; inset:0; background:rgba(99,102,241,.82); display:flex; flex-direction:column; align-items:center; justify-content:center; z-index:10; gap:8px; pointer-events:none; }
  .fm-drop-cover p { color:#fff; font-weight:600; font-size:14px; }
  .fm-loading { display:flex; flex-direction:column; align-items:center; justify-content:center; height:200px; color:var(--fm-muted); }
  .fm-spin { width:26px; height:26px; border:2.5px solid var(--fm-border); border-top-color:var(--fm-accent); border-radius:50%; animation:spin .65s linear infinite; }
  @keyframes spin { to { transform:rotate(360deg); } }
  .fm-empty { display:flex; flex-direction:column; align-items:center; justify-content:center; height:240px; color:var(--fm-muted); text-align:center; }
  .fm-empty-icon { width:64px; height:64px; border-radius:16px; background:rgba(99,102,241,.07); display:flex; align-items:center; justify-content:center; margin-bottom:12px; }
  .muted { color:var(--fm-muted); }

  /* List view */
  .fm-list-wrap { min-height:0; }
  .fm-table { width:100%; border-collapse:collapse; }
  .fm-thead-row th { padding:7px 8px; font-size:11px; font-weight:600; color:var(--fm-muted); text-transform:uppercase; letter-spacing:.05em; border-bottom:1px solid var(--fm-border); background:var(--fm-bg-side); text-align:left; white-space:nowrap; position:sticky; top:0; z-index:1; }
  .fm-thead-row th.sortable { cursor:pointer; user-select:none; }
  .fm-thead-row th.sortable:hover { color:var(--fm-text); }
  .sort-arrow { margin-left:3px; font-size:10px; }
  .tc-chk { width:34px; padding-left:12px !important; }
  .tc-name { min-width:180px; }
  .tc-size { width:80px; }
  .tc-date { width:155px; }
  .tc-perm { width:70px; font-family:monospace; font-size:11px; }
  .tc-act { width:120px; text-align:right; padding-right:10px !important; }
  .fm-row { border-bottom:1px solid rgba(0,0,0,.03); cursor:pointer; transition:background .08s; }
  :global(.dark) .fm-row { border-bottom-color:rgba(255,255,255,.03); }
  .fm-row:hover { background:var(--fm-bg-hover); }
  .fm-row td { padding:7px 8px; font-size:13px; color:var(--fm-text); vertical-align:middle; }
  .fm-row .tc-size, .fm-row .tc-date, .fm-row .tc-perm { color:var(--fm-muted); font-size:12px; }
  .fm-row-sel { background:var(--fm-bg-sel) !important; }
  .row-inner { display:flex; align-items:center; gap:7px; }
  .row-icon-wrap { flex-shrink:0; display:flex; align-items:center; }
  .row-name-text { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .row-name-text.is-dir { font-weight:500; }
  .inline-rename-inp { font-size:13px; padding:1px 5px; border:1.5px solid var(--fm-accent); border-radius:4px; background:var(--fm-bg); color:var(--fm-text); outline:none; min-width:80px; max-width:260px; }
  .row-acts { display:flex; gap:2px; justify-content:flex-end; opacity:0; transition:opacity .1s; }
  .fm-row:hover .row-acts { opacity:1; }
  .ra-btn { padding:3px; border-radius:4px; color:var(--fm-muted); transition:background .1s,color .1s; display:flex; align-items:center; cursor:pointer; border:none; background:transparent; }
  .ra-btn:hover { background:var(--fm-bg-hover); color:var(--fm-text); }
  .ra-del:hover { background:rgba(239,68,68,.12); color:var(--fm-danger); }

  /* Grid view */
  .fm-grid { display:grid; grid-template-columns:repeat(auto-fill,minmax(104px,1fr)); gap:6px; padding:10px; }
  .gc { display:flex; flex-direction:column; align-items:center; padding:10px 6px 8px; border-radius:9px; border:1.5px solid transparent; cursor:pointer; transition:background .1s,border-color .1s; position:relative; text-align:center; background:transparent; }
  .gc:hover { background:var(--fm-bg-hover); }
  .gc-sel { background:var(--fm-bg-sel) !important; border-color:var(--fm-accent) !important; }
  .gc-icon { margin-bottom:5px; }
  .gc-name { font-size:11.5px; color:var(--fm-text); overflow:hidden; text-overflow:ellipsis; white-space:nowrap; width:100%; max-width:88px; }
  .gc-size { font-size:10px; color:var(--fm-muted); margin-top:1px; }
  .gc-chk { position:absolute; top:3px; left:3px; opacity:0; transition:opacity .1s; }
  .gc:hover .gc-chk, .gc-sel .gc-chk { opacity:1; }

  /* Status bar */
  .fm-statusbar { display:flex; align-items:center; justify-content:space-between; padding:4px 12px; border-top:1px solid var(--fm-border); font-size:11px; background:var(--fm-bg-side); flex-shrink:0; }
  .fm-sb-l { display:flex; align-items:center; gap:10px; }
  .fm-sb-l span { color:var(--fm-muted); }
  .sb-sel { color:var(--fm-accent) !important; font-weight:500; }
  .sb-clip { color:#f59e0b !important; }
  .sb-muted { color:var(--fm-muted); }
  .fm-sb-r { color:var(--fm-muted); }

  /* Context menu */
  .fm-ctx { position:fixed; z-index:9999; background:var(--fm-bg); border:1px solid var(--fm-border); border-radius:10px; padding:4px; min-width:190px; box-shadow:0 8px 32px rgba(0,0,0,.18),0 2px 8px rgba(0,0,0,.08); }
  .ctx-it { display:flex; align-items:center; gap:8px; width:100%; padding:6px 10px; font-size:13px; color:var(--fm-text); border-radius:6px; text-align:left; cursor:pointer; border:none; background:transparent; transition:background .08s; }
  .ctx-it span:first-of-type { flex:1; }
  .ctx-it:hover { background:var(--fm-bg-hover); }
  .ctx-danger { color:var(--fm-danger); }
  .ctx-danger:hover { background:rgba(239,68,68,.12); }
  .ctx-sep { height:1px; background:var(--fm-border); margin:3px 4px; }
  .ctx-kbd { margin-left:auto; font-size:10px; font-family:monospace; color:var(--fm-muted); background:var(--fm-bg-side); border:1px solid var(--fm-border); border-radius:4px; padding:1px 5px; flex-shrink:0; white-space:nowrap; }
  .ctx-kbd-danger { color:rgba(239,68,68,.7); border-color:rgba(239,68,68,.3); background:rgba(239,68,68,.06); }

  /* Modals */
  .modal-overlay { position:fixed; inset:0; background:rgba(0,0,0,.5); display:flex; align-items:center; justify-content:center; z-index:2000; padding:16px; backdrop-filter:blur(2px); }
  .modal-box { background:var(--fm-bg); border-radius:14px; padding:24px; width:100%; max-width:380px; box-shadow:0 24px 64px rgba(0,0,0,.24); border:1px solid var(--fm-border); }
  .modal-title { font-size:15px; font-weight:700; color:var(--fm-text); margin-bottom:4px; display:flex; align-items:center; gap:6px; } .modal-icon { width:14px; height:14px; color:var(--fm-accent,#6366F1); flex-shrink:0; }
  .modal-sub { font-size:12px; color:var(--fm-muted); margin-bottom:14px; font-family:monospace; }
  .modal-label { display:block; font-size:11px; font-weight:600; color:var(--fm-muted); text-transform:uppercase; letter-spacing:.05em; margin-bottom:5px; }
  .modal-input { display:block; width:100%; padding:8px 10px; border-radius:8px; border:1px solid var(--fm-border); background:var(--fm-bg); font-size:13px; color:var(--fm-text); outline:none; box-sizing:border-box; }
  .modal-input:focus { border-color:var(--fm-accent); box-shadow:0 0 0 2px rgba(99,102,241,.15); }
  .modal-actions { display:flex; gap:8px; }
  .mb-primary { padding:8px 16px; border-radius:8px; background:var(--fm-accent); color:#fff; font-size:13px; font-weight:500; cursor:pointer; border:none; transition:opacity .1s; }
  .mb-primary:hover { opacity:.9; }
  .mb-primary:disabled { opacity:.5; cursor:not-allowed; }
  .mb-ghost { padding:8px 16px; border-radius:8px; border:1px solid var(--fm-border); color:var(--fm-muted); font-size:13px; cursor:pointer; background:transparent; transition:background .1s; }
  .mb-ghost:hover { background:var(--fm-bg-hover); }

  /* Chmod */
  .chmod-wrap { margin-bottom:4px; }
  .chmod-grid { display:grid; grid-template-columns:60px repeat(3,1fr); gap:6px 8px; align-items:center; margin-bottom:8px; }
  .cg-hdr { font-size:11px; font-weight:600; color:var(--fm-muted); text-align:center; }
  .cg-lbl { font-size:12px; color:var(--fm-text); }
  .cg-cell { display:flex; justify-content:center; }
  .chmod-row { display:flex; align-items:center; gap:8px; flex-wrap:wrap; }
  .chmod-pre { padding:3px 8px; border-radius:5px; border:1px solid var(--fm-border); font-size:11px; font-family:monospace; color:var(--fm-muted); cursor:pointer; background:var(--fm-bg); transition:background .1s; }
  .chmod-pre:hover { background:var(--fm-bg-hover); color:var(--fm-text); }
  .mt-3 { margin-top:12px; }
  .mt-4 { margin-top:16px; }
  .mb-3 { margin-bottom:12px; }

  /* Editor */
  .modal-editor { background:var(--fm-bg); border-radius:14px; width:min(96vw,1040px); height:86vh; display:flex; flex-direction:column; box-shadow:0 24px 64px rgba(0,0,0,.28); border:1px solid var(--fm-border); overflow:hidden; }
  .med-header { display:flex; align-items:center; justify-content:space-between; padding:12px 16px; border-bottom:1px solid var(--fm-border); flex-shrink:0; gap:8px; background:var(--fm-bg-side); }
  .med-title { display:flex; align-items:center; gap:7px; min-width:0; }
  .med-path { font-size:11px; color:var(--fm-muted); font-family:monospace; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
  .med-hint { padding:4px 16px; font-size:11px; color:var(--fm-muted); background:var(--fm-bg-side); border-bottom:1px solid var(--fm-border); flex-shrink:0; }
  .med-area { flex:1; padding:16px 20px; font-family:'JetBrains Mono','Fira Code','Cascadia Code',ui-monospace,monospace; font-size:13px; line-height:1.7; resize:none; border:none; outline:none; background:var(--fm-bg); color:var(--fm-text); tab-size:2; }

  /* Image preview */
  .modal-preview { background:var(--fm-bg); border-radius:14px; width:min(96vw,920px); max-height:92vh; display:flex; flex-direction:column; box-shadow:0 24px 64px rgba(0,0,0,.28); border:1px solid var(--fm-border); overflow:hidden; }
  .mp-header { display:flex; align-items:center; justify-content:space-between; padding:12px 16px; border-bottom:1px solid var(--fm-border); flex-shrink:0; background:var(--fm-bg-side); }
  .mp-body { flex:1; overflow:auto; display:flex; align-items:center; justify-content:center; padding:20px; background:var(--fm-bg-side); }
  .mp-img { max-width:100%; max-height:72vh; object-fit:contain; border-radius:8px; box-shadow:0 4px 24px rgba(0,0,0,.2); }

  /* Help */
  .help-grid { display:grid; grid-template-columns:auto 1fr; gap:6px 14px; align-items:start; }
  .hk { padding:2px 7px; border-radius:5px; border:1px solid var(--fm-border); font-family:monospace; font-size:11px; color:var(--fm-text); background:var(--fm-bg-side); white-space:nowrap; display:inline-block; }
  .hd { font-size:12px; color:var(--fm-muted); align-self:center; }

  /* Toast */
  .fm-toast { position:fixed; top:16px; right:16px; z-index:99999; padding:10px 18px; border-radius:10px; font-size:13px; font-weight:500; color:#fff; box-shadow:0 4px 24px rgba(0,0,0,.2); animation:toastIn .2s ease; pointer-events:none; }
  .toast-ok { background:#10b981; }
  .toast-err { background:var(--fm-danger); }
  @keyframes toastIn { from { transform:translateY(-8px); opacity:0; } to { transform:translateY(0); opacity:1; } }

  /* Properties modal */
  .prop-header { display:flex; align-items:center; gap:10px; margin-bottom:16px; }
  .prop-icon { flex-shrink:0; }
  .prop-grid { display:grid; grid-template-columns:90px 1fr; gap:8px 12px; align-items:start; }
  .prop-lbl { font-size:11px; font-weight:600; color:var(--fm-muted); text-transform:uppercase; letter-spacing:.05em; padding-top:1px; }
  .prop-val { font-size:13px; color:var(--fm-text); word-break:break-all; }
  .prop-mono { font-family:monospace; font-size:12px; }
  .prop-muted { color:var(--fm-muted); }
</style>
