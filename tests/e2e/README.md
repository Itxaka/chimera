# End-to-End Tests

This directory contains manual procedures for testing Chimera's core functionality.

## Prerequisites

Before running any E2E tests, ensure the following are installed and configured:

1. **Kernel & KVM**
   - Linux kernel with KVM support
   - `/dev/kvm` accessible (add your user to the `kvm` group)
   - Verify with: `ls -l /dev/kvm`

2. **cloud-hypervisor**
   - Installed and on your PATH
   - Verify with: `which cloud-hypervisor`

3. **Networking**
   - A Linux bridge configured (e.g., `br0`)
   - iproute2 installed (`ip` command)
   - Verify with: `ip link show br0`

4. **Privileged helper**
   - `chimera-netd` installed at `/usr/libexec/chimera-netd`
   - polkit policy installed at `/usr/share/polkit-1/actions/org.chimera.netd.policy`
   - Verify with: `ls -l /usr/libexec/chimera-netd`

5. **Test artifacts**
   - A bootable firmware disk image (e.g., `firmware.img`)
   - A firmware binary (e.g., `OVMF.fd`)
   - Verify paths are set in environment variables

## Running Gated E2E Tests

The gated E2E test (`create_boot_stop_roundtrip`) documents the full lifecycle: VM creation → boot → stop → cleanup.

### Setup environment variables

```bash
export CHIMERA_TEST_DISK=/path/to/firmware.img
export CHIMERA_TEST_FW=/path/to/OVMF.fd
export CHIMERA_TEST_BRIDGE=br0
```

### Run the test

```bash
cargo test -p chimera-core --test e2e_create -- --ignored --nocapture
```

This will:
1. Create a new VM definition with 1 vCPU, 512 MiB RAM
2. Boot the VM via the Manager
3. Verify the VM status is `Running`
4. Stop the VM
5. Clean up the VM from persistent storage

## Troubleshooting

- **`/dev/kvm` permission denied**: Add your user to the `kvm` group with `sudo usermod -a -G kvm $USER` and log out/in.
- **`cloud-hypervisor` not found**: Install cloud-hypervisor and ensure it is on your PATH.
- **Bridge not found**: Create a bridge with `sudo ip link add br0 type bridge` and bring it up with `sudo ip link set br0 up`.
- **polkit policy denied**: Ensure the policy file is installed and readable at `/usr/share/polkit-1/actions/org.chimera.netd.policy`.

## Manual Integration Flow (reference)

If you want to manually test the Chimera workflow without the automated test:

1. **Create a VM definition** with desired vCPU, memory, disk, and network configuration
2. **Boot the VM** via the Manager, which:
   - Allocates a tap device for networking
   - Bridges it to your configured network bridge
   - Starts cloud-hypervisor with the firmware disk
3. **Monitor the VM** via the dashboard or CLI
4. **Stop the VM** when complete, which:
   - Sends a shutdown signal to cloud-hypervisor
   - Tears down the tap device
   - Cleans up socket and pidfiles
5. **Delete the VM definition** to remove persistent state
