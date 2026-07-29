<script lang="ts">
  import { t } from '$lib/i18n';
  export let status: 'active' | 'suspended' | 'provisioning' | 'error' | 'pending' | 'expired';

  const config: Record<string, { label: string; classes: string }> = {
    active:       { label: 'Active',       classes: 'bg-green-100  text-green-800  dark:bg-green-900/30  dark:text-green-400' },
    suspended:    { label: 'Suspended',    classes: 'bg-amber-100  text-amber-800  dark:bg-amber-900/30  dark:text-amber-400' },
    provisioning: { label: 'Provisioning', classes: 'bg-blue-100   text-blue-800   dark:bg-blue-900/30   dark:text-blue-400'  },
    error:        { label: 'Error',        classes: 'bg-red-100    text-red-800    dark:bg-red-900/30    dark:text-red-400'   },
    pending:      { label: 'Pending',      classes: 'bg-slate-100  text-slate-700  dark:bg-slate-700/40  dark:text-slate-300' },
    expired:      { label: 'Expired',      classes: 'bg-orange-100 text-orange-800 dark:bg-orange-900/30 dark:text-orange-400'},
  };

  const dotColors: Record<string, string> = {
    active:       'bg-green-500',
    suspended:    'bg-amber-500',
    provisioning: 'bg-blue-500',
    error:        'bg-red-500',
    pending:      'bg-slate-400',
    expired:      'bg-orange-500',
  };

  $: cfg = config[status] ?? { label: status, classes: 'bg-muted text-muted-foreground' };
  $: dotColor = dotColors[status] ?? 'bg-muted-foreground';
</script>

<span class="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium {cfg.classes}">
  {#if status === 'active'}
    <!-- Pulsing ping dot for active -->
    <span class="relative flex h-1.5 w-1.5 shrink-0">
      <span class="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"></span>
      <span class="relative inline-flex rounded-full h-1.5 w-1.5 bg-green-400"></span>
    </span>
  {:else if status === 'provisioning' || status === 'pending'}
    <!-- Spinning loader for in-progress states -->
    <svg class="w-3 h-3 animate-spin shrink-0" fill="none" viewBox="0 0 24 24">
      <circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="2.5" stroke-dasharray="40 14" fill="none"/>
    </svg>
  {:else}
    <!-- Static dot for all other states -->
    <span class="w-1.5 h-1.5 rounded-full shrink-0 {dotColor}"></span>
  {/if}
  {cfg.label}
</span>
