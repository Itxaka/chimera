<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { listVms } from '$lib/api';
  import VmList from '$lib/components/VmList.svelte';
  import CreateWizard from '$lib/components/CreateWizard.svelte';

  let vms = [];
  let error = '';
  let showWizard = false;
  let timer;

  async function refresh() {
    try { vms = await listVms(); error = ''; }
    catch (e) { error = String(e); }
  }

  onMount(() => { refresh(); timer = setInterval(refresh, 3000); });
  onDestroy(() => clearInterval(timer));
</script>

<header>
  <h1>Chimera</h1>
  <button on:click={() => (showWizard = true)}>+ New VM</button>
</header>

{#if error}<p class="err">{error}</p>{/if}

<VmList {vms} onChange={refresh} />

{#if showWizard}
  <CreateWizard on:close={() => (showWizard = false)} on:created={refresh} />
{/if}

<style>
  header { display: flex; justify-content: space-between; align-items: center; }
  .err { color: #b00; }
</style>
