<script lang="ts">
  export let size: number = 24;
  export let className: string = '';
</script>

<!-- JottiCP brand icon: planet + tilted orbit ring + satellite -->
<svg
  class={className}
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  aria-hidden="true"
>
  <!-- Planet core -->
  <circle cx="12" cy="12" r="2.8" fill="currentColor" />
  <!-- Orbit ring (tilted ellipse -30°) -->
  <ellipse
    cx="12" cy="12"
    rx="8" ry="3.2"
    stroke="currentColor"
    stroke-width="1.5"
    fill="none"
    transform="rotate(-30 12 12)"
  />
  <!-- Satellite on the orbit ring -->
  <circle cx="18.9" cy="8.0" r="1.4" fill="currentColor" />
</svg>
