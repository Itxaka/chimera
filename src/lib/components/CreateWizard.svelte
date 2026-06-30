<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  const dispatch = createEventDispatcher();
</script>

<div class="modal-overlay">
  <div class="modal">
    <h2>Create VM</h2>
    <p>CreateWizard stub — to be implemented in Task 11.</p>
    <button on:click={() => dispatch('close')}>Cancel</button>
  </div>
</div>

<style>
  .modal-overlay { position: fixed; top: 0; left: 0; width: 100%; height: 100%; background: rgba(0, 0, 0, 0.5); display: flex; align-items: center; justify-content: center; }
  .modal { background: white; padding: 2rem; border-radius: 8px; box-shadow: 0 2px 10px rgba(0, 0, 0, 0.1); }
</style>
