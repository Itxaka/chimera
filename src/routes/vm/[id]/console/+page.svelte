<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { page } from '$app/stores';
  import { listen } from '@tauri-apps/api/event';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { openConsole, consoleInput, closeConsole, consoleLogPath } from '$lib/api';

  $: id = $page.params.id;

  let el;
  let term;
  let fit;
  let unlisten;
  let logPath = '';
  const enc = new TextEncoder();

  onMount(async () => {
    term = new Terminal({ convertEol: true, fontFamily: 'ui-monospace, monospace', fontSize: 13 });
    fit = new FitAddon();
    term.loadAddon(fit);
    term.open(el);
    fit.fit();

    // last ~4KB of history for context
    const tail = await openConsole(id);
    if (tail && tail.length) term.write(new Uint8Array(tail));

    // live stream
    unlisten = await listen('console-data', (e) => {
      const p = e.payload;
      if (p && p.id === id) term.write(new Uint8Array(p.bytes));
    });

    // keystrokes -> guest
    term.onData((d) => consoleInput(id, Array.from(enc.encode(d))));

    try { logPath = await consoleLogPath(id); } catch { logPath = ''; }
  });

  onDestroy(() => {
    if (unlisten) unlisten();
    closeConsole(id);
    if (term) term.dispose();
  });
</script>

<div class="bar">
  <a href="/vm/{id}">← Back</a>
  {#if logPath}<span class="logpath" title={logPath}>log: {logPath}</span>{/if}
</div>
<div class="term" bind:this={el}></div>

<style>
  .bar { display: flex; gap: 1rem; align-items: center; margin-bottom: 0.5rem; }
  .logpath { color: #666; font-size: 0.75rem; font-family: ui-monospace, monospace; }
  .term { height: calc(100vh - 4rem); background: #000; padding: 0.25rem; }
</style>
