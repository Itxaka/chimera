<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { page } from '$app/stores';
  import { goto } from '$app/navigation';
  import { listVms, startVm, stopVm, pauseVm, resumeVm, deleteVm } from '$lib/api';

  $: id = $page.params.id;

  let vm = null;
  let loaded = false;
  let error = '';
  let busy = false;
  let timer;

  async function refresh() {
    try {
      const all = await listVms();
      vm = all.find((v) => v.definition.id === id) ?? null;
      loaded = true;
      error = '';
    } catch (e) {
      error = String(e);
    }
  }

  async function run(fn) {
    busy = true;
    try {
      await fn();
    } catch (e) {
      alert(String(e));
    } finally {
      busy = false;
      refresh();
    }
  }

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 3000);
  });
  onDestroy(() => clearInterval(timer));
</script>

<a class="back" href="/">← Back to dashboard</a>

{#if error}<p class="err">{error}</p>{/if}

{#if loaded && !vm}
  <p>VM <code>{id}</code> not found.</p>
{:else if vm}
  <header>
    <h1>{vm.definition.name}</h1>
    <span class="status {vm.runtime.status}">{vm.runtime.status}</span>
  </header>

  {#if vm.runtime.last_error}
    <p class="err">⚠ {vm.runtime.last_error}</p>
  {/if}

  <div class="actions">
    {#if vm.runtime.status === 'running'}
      <button disabled={busy} on:click={() => run(() => stopVm(id))}>Stop</button>
      <button disabled={busy} on:click={() => run(() => pauseVm(id))}>Pause</button>
    {:else if vm.runtime.status === 'paused'}
      <button disabled={busy} on:click={() => run(() => resumeVm(id))}>Resume</button>
      <button disabled={busy} on:click={() => run(() => stopVm(id))}>Stop</button>
    {:else}
      <button disabled={busy} on:click={() => run(() => startVm(id))}>Start</button>
    {/if}
    <button
      disabled={busy}
      on:click={() => run(async () => { await deleteVm(id); goto('/'); })}>Delete</button
    >
  </div>

  <h2>Definition</h2>
  <dl>
    <dt>id</dt><dd><code>{vm.definition.id}</code></dd>
    <dt>vCPUs</dt><dd>{vm.definition.vcpus}</dd>
    <dt>Memory</dt><dd>{vm.definition.memory_mib} MiB</dd>
    <dt>Firmware</dt><dd><code>{vm.definition.boot.firmware}</code></dd>
    <dt>Bridge</dt><dd>{vm.definition.net.bridge}</dd>
    <dt>Created</dt><dd>{vm.definition.created_at}</dd>
  </dl>

  <h3>Disks</h3>
  <ul>
    {#each vm.definition.disks as d}
      <li><code>{d.path}</code>{d.readonly ? ' (read-only)' : ''}</li>
    {/each}
  </ul>

  <h2>Runtime</h2>
  <dl>
    <dt>pid</dt><dd>{vm.runtime.pid ?? '—'}</dd>
    <dt>socket</dt><dd><code>{vm.runtime.socket}</code></dd>
    <dt>tap</dt><dd>{vm.runtime.tap ?? '—'}</dd>
  </dl>
{:else}
  <p>Loading…</p>
{/if}

<style>
  .back { display: inline-block; margin-bottom: 1rem; }
  header { display: flex; align-items: center; gap: 0.75rem; }
  .status { padding: 0.1rem 0.5rem; border-radius: 4px; font-size: 0.85rem; }
  .running { background: #d1f7d1; }
  .stopped { background: #eee; }
  .paused { background: #fff0c0; }
  .failed { background: #f7d1d1; }
  .creating { background: #d1e7f7; }
  .err { color: #b00; }
  .actions { display: flex; gap: 0.5rem; margin: 1rem 0; }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: 0.25rem 1rem; }
  dt { font-weight: 600; }
  dd { margin: 0; }
  code { font-family: ui-monospace, monospace; font-size: 0.9em; }
</style>
