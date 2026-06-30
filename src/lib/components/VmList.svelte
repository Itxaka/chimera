<script lang="ts">
  import VmRow from './VmRow.svelte';

  export let vms;
  export let onChange;
</script>

{#if vms.length === 0}
  <p>No VMs yet. Create one to get started.</p>
{:else}
  <table>
    <thead><tr><th>Name</th><th>Status</th><th>vCPU</th><th>Memory</th><th>Actions</th></tr></thead>
    <tbody>
      {#each vms as vm (vm.definition.id)}
        <VmRow {vm} {onChange} />
      {/each}
    </tbody>
  </table>
{/if}

<style>
  table { width: 100%; border-collapse: collapse; }
  th, td { text-align: left; padding: 0.4rem; border-bottom: 1px solid #eee; }
</style>
