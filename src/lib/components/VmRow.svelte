<script lang="ts">
  import { startVm, stopVm, pauseVm, resumeVm, deleteVm } from '$lib/api';

  export let vm;
  export let onChange;

  let busy = false;
  async function run(fn) {
    busy = true;
    try { await fn(); } catch (e) { alert(String(e)); } finally { busy = false; onChange(); }
  }
  $: status = vm.runtime.status;
</script>

<tr>
  <td>{vm.definition.name}</td>
  <td><span class="status {status}">{status}</span></td>
  <td>{vm.definition.vcpus}</td>
  <td>{vm.definition.memory_mib} MiB</td>
  <td>
    {#if status === 'running'}
      <button disabled={busy} on:click={() => run(() => stopVm(vm.definition.id))}>Stop</button>
      <button disabled={busy} on:click={() => run(() => pauseVm(vm.definition.id))}>Pause</button>
    {:else if status === 'paused'}
      <button disabled={busy} on:click={() => run(() => resumeVm(vm.definition.id))}>Resume</button>
      <button disabled={busy} on:click={() => run(() => stopVm(vm.definition.id))}>Stop</button>
    {:else}
      <button disabled={busy} on:click={() => run(() => startVm(vm.definition.id))}>Start</button>
    {/if}
    <button disabled={busy} on:click={() => run(() => deleteVm(vm.definition.id))}>Delete</button>
  </td>
</tr>

<style>
  .status { padding: 0.1rem 0.4rem; border-radius: 4px; font-size: 0.8rem; }
  .running { background: #d1f7d1; }
  .stopped { background: #eee; }
  .paused  { background: #fff0c0; }
  .failed  { background: #f7d1d1; }
  .creating{ background: #d1e7f7; }
</style>
