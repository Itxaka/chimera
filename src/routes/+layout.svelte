<script lang="ts">
  // SPA: disable SSR; Tauri serves static files.
  export const ssr = false;
  export const prerender = true;
</script>

<main>
  <slot />
</main>

<style>
  main { font-family: system-ui, sans-serif; padding: 1rem; }
</style>
