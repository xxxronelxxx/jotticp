<script lang="ts">
  import HelpTip from './HelpTip.svelte';

  export let title: string;
  export let subtitle: string = '';
  export let helpTitle: string = '';
  export let helpContent: string = '';
  /** Optional Heroicons SVG path string — shows an icon badge before the title */
  export let icon: string = '';
</script>

<div class="flex items-start gap-3">
  <!-- Optional icon badge -->
  {#if icon}
    <div class="w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center mr-1 shrink-0">
      <svg class="w-5 h-5 text-primary" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="1.75">
        <path stroke-linecap="round" stroke-linejoin="round" d={icon}/>
      </svg>
    </div>
  {/if}

  <div class="flex-1 min-w-0">
    <div class="flex items-center gap-2">
      <h1 class="text-2xl font-semibold text-[var(--text-primary)] truncate">{title}</h1>
      {#if helpTitle && helpContent}
        <HelpTip title={helpTitle} content={helpContent} />
      {/if}
    </div>
    {#if subtitle}
      <p class="text-sm text-[var(--text-muted)] mt-0.5">{subtitle}</p>
    {/if}
  </div>
  <!-- Slot for action buttons on the right -->
  <div class="flex items-center gap-2 shrink-0 pt-0.5">
    <slot />
  </div>
</div>

<style>
  /* Smooth underline animation for any breadcrumb / link children */
  :global(.page-header-breadcrumb a) {
    position: relative;
    text-decoration: none;
  }
  :global(.page-header-breadcrumb a::after) {
    content: '';
    position: absolute;
    bottom: -1px;
    left: 0;
    width: 0;
    height: 1px;
    background: currentColor;
    transition: width 0.2s ease;
  }
  :global(.page-header-breadcrumb a:hover::after) {
    width: 100%;
  }
</style>
