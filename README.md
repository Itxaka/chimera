# Chimera

Desktop fleet manager for cloud-hypervisor VMs.

## Prerequisites (dev, Linux)
- Rust stable, Node 20+
- `cloud-hypervisor` on PATH
- `/dev/kvm` accessible (add your user to the `kvm` group)
- System libs: `webkit2gtk-4.1`, `libsoup-3.0`, `libappindicator` dev packages
- polkit (`pkexec`), iproute2 (`ip`)
- An existing Linux bridge (e.g. `br0`)

## Install the privileged helper (required for networking)
```sh
cargo build -p chimera-netd --release
sudo install -m 0755 target/release/chimera-netd /usr/libexec/chimera-netd
sudo install -m 0644 packaging/org.chimera.netd.policy /usr/share/polkit-1/actions/
```

## Run (dev)
```sh
npm install
npm run tauri dev
```

## Architecture
See `docs/superpowers/specs/2026-06-30-chimera-v0.1-design.md` and
`docs/superpowers/plans/2026-06-30-chimera-v0.1.md`.

## State
VM definitions: `~/.config/chimera/vms/<id>/{definition,runtime}.toml`
Sockets + pidfiles: `$XDG_RUNTIME_DIR/chimera/<id>.{sock,pid}`
