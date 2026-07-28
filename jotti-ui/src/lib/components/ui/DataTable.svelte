<script lang="ts">
  export let columns: Array<{ key: string; label: string; sortable?: boolean; class?: string }> = [];
  export let rows: Record<string, unknown>[] = [];
  export let loading: boolean = false;
  export let emptyMessage: string = 'No data to display.';
  export let selectable: boolean = false;

  let sortKey: string = '';
  let sortDir: 'asc' | 'desc' = 'asc';
  let selectedIds = new Set<number>();

  function handleSort(key: string) {
    if (!columns.find(c => c.key === key)?.sortable) return;
    if (sortKey === key) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    } else {
      sortKey = key;
      sortDir = 'asc';
    }
  }

  $: sortedRows = (() => {
    if (!sortKey) return rows;
    return [...rows].sort((a, b) => {
      const av = a[sortKey];
      const bv = b[sortKey];
      if (av == null && bv == null) return 0;
      if (av == null) return 1;
      if (bv == null) return -1;
      const cmp = String(av).localeCompare(String(bv), undefined, { numeric: true });
      return sortDir === 'asc' ? cmp : -cmp;
    });
  })();

  function toggleAll() {
    if (selectedIds.size === rows.length) {
      selectedIds = new Set();
    } else {
      selectedIds = new Set(rows.map((_, i) => i));
    }
  }

  function toggleRow(idx: number) {
    const next = new Set(selectedIds);
    if (next.has(idx)) {
      next.delete(idx);
    } else {
      next.add(idx);
    }
    selectedIds = next;
  }

  $: allSelected = rows.length > 0 && selectedIds.size === rows.length;
  $: someSelected = selectedIds.size > 0 && !allSelected;

  // Expose selection to parent
  $: selectedRows = [...selectedIds].map(i => sortedRows[i]).filter(Boolean);

  // Dispatch selection change
  import { createEventDispatcher } from 'svelte';
  const dispatch = createEventDispatcher<{ select: typeof selectedRows }>();
  $: dispatch('select', selectedRows);
</script>

<style>
  .row-enter {
    animation: rowFadeIn 0.15s ease-out both;
  }
  @keyframes rowFadeIn {
    from { opacity: 0; transform: translateY(3px); }
    to   { opacity: 1; transform: none; }
  }
</style>

<div class="bg-card border border-border rounded-xl overflow-hidden">
  <div class="overflow-x-auto">
    <table class="w-full text-sm">
      <thead>
        <tr class="border-b border-border text-left">
          {#if selectable}
            <th class="px-4 py-3 w-10">
              <input
                type="checkbox"
                checked={allSelected}
                indeterminate={someSelected}
                on:change={toggleAll}
                aria-label="Select all rows"
                class="rounded border-border text-primary focus:ring-ring"
              />
            </th>
          {/if}
          {#each columns as col}
            <th
              class="px-4 py-3 text-xs font-medium text-muted-foreground uppercase tracking-wider
                     whitespace-nowrap {col.class ?? ''} {col.sortable ? 'cursor-pointer select-none hover:text-foreground' : ''}"
              on:click={() => col.sortable && handleSort(col.key)}
            >
              <span class="inline-flex items-center gap-1">
                {col.label}
                {#if col.sortable}
                  <span class="text-[10px] {sortKey === col.key ? 'text-primary' : 'text-muted-foreground/40'}">
                    {#if sortKey === col.key && sortDir === 'asc'}
                      <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 20 20">
                        <path fill-rule="evenodd" d="M14.707 12.707a1 1 0 01-1.414 0L10 9.414l-3.293 3.293a1 1 0 01-1.414-1.414l4-4a1 1 0 011.414 0l4 4a1 1 0 010 1.414z" clip-rule="evenodd" />
                      </svg>
                    {:else if sortKey === col.key && sortDir === 'desc'}
                      <svg class="w-3 h-3" fill="currentColor" viewBox="0 0 20 20">
                        <path fill-rule="evenodd" d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z" clip-rule="evenodd" />
                      </svg>
                    {:else}
                      <svg class="w-3 h-3 opacity-30" fill="currentColor" viewBox="0 0 20 20">
                        <path d="M5 10a1 1 0 011-1h8a1 1 0 110 2H6a1 1 0 01-1-1z" />
                      </svg>
                    {/if}
                  </span>
                {/if}
              </span>
            </th>
          {/each}
        </tr>
      </thead>

      <tbody class="divide-y divide-border">
        {#if loading}
          <!-- Skeleton rows -->
          {#each [1, 2, 3, 4, 5] as _}
            <tr>
              {#if selectable}
                <td class="px-4 py-3"><div class="w-4 h-4 bg-muted rounded animate-pulse"></div></td>
              {/if}
              {#each columns as _col}
                <td class="px-4 py-3">
                  <div class="h-4 bg-muted rounded animate-pulse" style="width: {60 + Math.random() * 30}%"></div>
                </td>
              {/each}
            </tr>
          {/each}
        {:else if sortedRows.length === 0}
          <tr>
            <td
              colspan={columns.length + (selectable ? 1 : 0)}
              class="px-4 py-16 text-center"
            >
              <!-- Empty state illustration -->
              <div class="flex flex-col items-center gap-3">
                <svg class="w-16 h-16 text-muted-foreground/25" fill="none" viewBox="0 0 64 64" stroke="currentColor" stroke-width="1.25">
                  <!-- Inbox tray -->
                  <rect x="8" y="28" width="48" height="28" rx="4"/>
                  <path d="M8 40h12l4 6h16l4-6h12"/>
                  <!-- Magnifying glass -->
                  <circle cx="44" cy="18" r="10"/>
                  <path d="M51 25l7 7" stroke-linecap="round"/>
                  <!-- Lines suggesting content -->
                  <path d="M18 20h16M18 26h10" stroke-linecap="round" opacity="0.5"/>
                </svg>
                <p class="text-sm font-medium text-muted-foreground">{emptyMessage}</p>
              </div>
            </td>
          </tr>
        {:else}
          {#each sortedRows as row, i}
            <tr class="hover:bg-muted/30 transition-colors row-enter {selectedIds.has(i) ? 'bg-primary/5' : ''}">
              {#if selectable}
                <td class="px-4 py-3">
                  <input
                    type="checkbox"
                    checked={selectedIds.has(i)}
                    on:change={() => toggleRow(i)}
                    aria-label="Select row {i + 1}"
                    class="rounded border-border text-primary focus:ring-ring"
                  />
                </td>
              {/if}
              {#each columns as col}
                <td class="px-4 py-3 {col.class ?? ''}">
                  <slot name="cell" {col} {row} rowIndex={i}>
                    <span class="text-foreground">{row[col.key] ?? '—'}</span>
                  </slot>
                </td>
              {/each}
            </tr>
          {/each}
        {/if}
      </tbody>
    </table>
  </div>
</div>
