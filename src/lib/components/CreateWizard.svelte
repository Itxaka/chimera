<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import { createVm } from '$lib/api';

  const dispatch = createEventDispatcher();

  let name = '';
  let vcpus = 2;
  let memory_mib = 2048;
  let disk_path = '';
  let firmware_path = '/usr/share/cloud-hypervisor/CLOUDHV.fd';
  let bridge = 'br0';
  let busy = false;
  let error = '';

  function validate() {
    if (!name.trim()) return 'Name is required';
    if (vcpus < 1 || vcpus > 64) return 'vCPUs must be 1–64';
    if (memory_mib < 128) return 'Memory must be ≥ 128 MiB';
    if (!disk_path.trim()) return 'Disk image path is required';
    if (!firmware_path.trim()) return 'Firmware path is required';
    if (!bridge.trim()) return 'Bridge is required';
    return null;
  }

  async function submit() {
    const v = validate();
    if (v) { error = v; return; }
    busy = true; error = '';
    const req = { name, vcpus, memory_mib, disk_path, firmware_path, bridge };
    try {
      await createVm(req);
      dispatch('created');
      dispatch('close');
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="backdrop" on:click={() => dispatch('close')}>
  <div class="modal" on:click|stopPropagation>
    <h2>Create VM</h2>
    {#if error}<p class="err">{error}</p>{/if}
    <label>Name <input bind:value={name} /></label>
    <label>vCPUs <input type="number" bind:value={vcpus} min="1" max="64" /></label>
    <label>Memory (MiB) <input type="number" bind:value={memory_mib} min="128" /></label>
    <label>Disk image path <input bind:value={disk_path} placeholder="/var/lib/images/disk.raw" /></label>
    <label>Firmware path <input bind:value={firmware_path} /></label>
    <label>Bridge <input bind:value={bridge} /></label>
    <div class="actions">
      <button on:click={() => dispatch('close')} disabled={busy}>Cancel</button>
      <button on:click={submit} disabled={busy}>{busy ? 'Creating…' : 'Create'}</button>
    </div>
  </div>
</div>

<style>
  .backdrop { position: fixed; inset: 0; background: rgba(0,0,0,.4); display: grid; place-items: center; }
  .modal { background: #fff; padding: 1.5rem; border-radius: 8px; width: 420px; display: flex; flex-direction: column; gap: .6rem; }
  label { display: flex; justify-content: space-between; gap: 1rem; align-items: center; }
  input { flex: 1; }
  .actions { display: flex; justify-content: flex-end; gap: .5rem; margin-top: 1rem; }
  .err { color: #b00; }
</style>
