# Chimera NAT networking + ch-log capture + independent consoles (design)

Date: 2026-07-01
Status: approved (brainstorming)

## Summary

Three usability fixes, one spec:

1. **NAT networking (C)** — the managed Chimera bridge becomes a NAT network
   (like libvirt's `virbr0`): gateway IP on the bridge, `ip_forward`, a
   masquerade firewall rule, and a dnsmasq DHCP+DNS server bound to the bridge.
   Guests get a `192.168.100.x` lease and internet. `chimera-netd` gains
   `net-up`/`net-down`; bridge setup/teardown drive them.
2. **cloud-hypervisor log capture (A)** — ch is spawned detached, so its
   `<vmm> WARN` output was inheriting the GUI's terminal and getting lost.
   Redirect ch stdout+stderr into the app log file so those lines land in
   `chimera.log` beside the `tracing` events.
3. **Independent console windows (B)** — the serial console opens as a
   standalone `adw::Window` per VM (several at once), instead of a single
   pushed navigation page.

## Background: what qemu/libvirt do

Bare qemu offers user-mode SLIRP (`-netdev user`: built-in NAT/DHCP/DNS,
unprivileged, no inbound) or tap+bridge (setuid `qemu-bridge-helper`) for L2 —
qemu gives **no** DHCP/NAT on a bridge. The NAT layer (bridge IP + dnsmasq +
`ip_forward` + iptables masquerade) is **libvirt's** default network. Approach 1
clones that design.

## Decisions (locked)

| Topic | Decision |
|-------|----------|
| Approach | Self-contained NAT in `chimera-netd` (not libvirt, not passt). |
| Subnet | `192.168.100.0/24`; gateway `192.168.100.1`; DHCP `192.168.100.2`–`192.168.100.254`, 12h lease. Avoids libvirt's `.122`. |
| Firewall | Prefer **nftables** (dedicated table `inet chimera_nat`); fall back to **iptables** if `nft` is absent. Detection happens root-side in netd. |
| dnsmasq | Required runtime dep. Bound to the bridge only; pidfile under the run dir; DNS forwards to system upstream. |
| Coupling | NAT is tied to the managed bridge: `setup_bridge` → `net-up`, `remove_bridge` → `net-down`. No separate toggle. |
| ip_forward | Runtime `sysctl -w net.ipv4.ip_forward=1` on `net-up` (not persisted). |
| ch log | ch stdout+stderr redirected (append) to `logging::log_path()`; interleaves with tracing at line granularity (O_APPEND, acceptable). |
| Console window | One standalone `adw::Window` per `OpenConsole`; kept in a `Vec`, dropped on window close. |

## Component changes

### C — NAT networking

**`chimera-netd` (`netops.rs`)** — pure command builders (testable), plus a
firewall enum:

```rust
pub enum Firewall { Nft, Iptables }

pub struct NatParams<'a> {
    pub bridge: &'a str,
    pub gateway: &'a str,   // "192.168.100.1"
    pub prefix: u8,         // 24
    pub cidr: &'a str,      // "192.168.100.0/24"
}

// address + forwarding + masquerade (firewall-specific). Idempotent:
// flush addr first; nft `add table` is idempotent, iptables uses check-then-add.
pub fn net_up_cmds(p: &NatParams, fw: Firewall) -> Vec<Vec<String>>;
// teardown: delete nft table (|| true) or flush iptables rules; flush bridge addr.
pub fn net_down_cmds(bridge: &str, cidr: &str, fw: Firewall) -> Vec<Vec<String>>;
// dnsmasq invocation (spawned, daemonizes): interface/bind/dhcp-range/pid-file.
pub fn dnsmasq_cmd(bridge: &str, gateway: &str, dhcp_lo: &str, dhcp_hi: &str, pidfile: &str) -> Vec<String>;

pub fn detect_firewall() -> Firewall; // `which nft` -> Nft else Iptables
```

`net_up_cmds` (nft variant) emits, in order:
1. `ip addr flush dev <br>`
2. `ip addr add <gw>/<prefix> dev <br>`
3. `sysctl -w net.ipv4.ip_forward=1`
4. `nft add table inet chimera_nat`
5. `nft add chain inet chimera_nat postrouting { type nat hook postrouting priority 100 ; }`
6. `nft add rule inet chimera_nat postrouting ip saddr <cidr> oifname != "<br>" masquerade`
7. `nft add chain inet chimera_nat forward { type filter hook forward priority 0 ; }`
8. `nft add rule inet chimera_nat forward iifname "<br>" accept`
9. `nft add rule inet chimera_nat forward oifname "<br>" ct state related,established accept`

iptables variant emits the equivalent (`iptables -w -t nat -A POSTROUTING -s
<cidr> ! -o <br> -j MASQUERADE`, `iptables -w -A FORWARD -i <br> -j ACCEPT`,
`iptables -w -A FORWARD -o <br> -m state --state RELATED,ESTABLISHED -j ACCEPT`,
each guarded by a `-C` check via `sh -c '... || ...'`).

`net_down_cmds` (nft): `nft delete table inet chimera_nat` (tolerated if
absent), then `ip addr flush dev <br>`. iptables: delete the three rules with
`-D` (tolerated), then flush addr.

**`chimera-netd` (`main.rs`)** — new verbs:
- `net-up --bridge <b> --gateway <g> --prefix <p> --cidr <c> --dhcp-lo <l> --dhcp-hi <h> --pidfile <f>`:
  `detect_firewall()`, `run_cmds(net_up_cmds(...))`, then spawn `dnsmasq_cmd(...)`
  (dnsmasq daemonizes, so the spawn returns).
- `net-down --bridge <b> --cidr <c> --pidfile <f>`: read `<pidfile>`, `kill`
  the pid if present (tolerated), then `detect_firewall()` +
  `run_cmds(net_down_cmds(...))`.

**GUI `setup.rs`** — the managed bridge carries a fixed NAT config:
- `const NAT_GATEWAY = "192.168.100.1"`, `NAT_PREFIX = 24`, `NAT_CIDR =
  "192.168.100.0/24"`, `NAT_DHCP_LO/HI`, and `nat_pidfile(bridge) ->
  <run_dir>/dnsmasq-<bridge>.pid`.
- `net_up_argv(bridge)` / `net_down_argv(bridge)` — build the `pkexec
  /usr/libexec/chimera-netd net-up|net-down ...` argv (absolute path per the
  netd path rule).
- `setup_bridge` runs bridge-create as today, **then** `net_up`. `remove_bridge`
  runs `net_down` **then** bridge-delete. Both `valid_ifname`-guarded.
- `doctor` reports: `dnsmasq: present/missing`, `ip_forward: 0/1`
  (`/proc/sys/net/ipv4/ip_forward`), and the managed bridge's IP
  (`ip -o addr show <br>`).

The polkit policy already grants `org.chimera.netd.manage` for any netd
invocation, so `net-up`/`net-down` authorize under the existing rule (no policy
change).

### A — cloud-hypervisor log capture

**`chimera-core` `supervisor.rs`** — `Supervisor` gains an optional log path:
- `pub fn with_log(run_dir: PathBuf, log_path: Option<PathBuf>) -> Self` (and
  keep `new(run_dir)` = `with_log(run_dir, None)`).
- In `spawn`, if `log_path` is set, open it with
  `OpenOptions::new().create(true).append(true)`, and set both
  `cmd.stdout(Stdio::from(file.try_clone()?))` and `cmd.stderr(Stdio::from(file))`.
  If `None`, inherit as before.

**GUI `dashboard.rs`** — `make_manager` builds the Manager with a Supervisor
constructed via `Supervisor::with_log(Supervisor::default_run_dir(),
Some(crate::logging::log_path()))` so ch output lands in `chimera.log`. (The
`Manager` constructor already takes a Supervisor; thread the log path there.)

### B — independent console windows

**GUI `app.rs`** — replace the single `console: Option<Controller<Console>>` +
`nav.push` with a `Vec<Controller<Console>>` (kept alive):
- `AppMsg::OpenConsole(id)`: launch a `Console` controller, wrap its widget in a
  new `adw::Window` (title `Console — <name/id>`, default size ~800×500,
  transient-for the main window but not modal), `window.present()`, push the
  controller into the Vec.
- On window `close-request`, remove the controller from the Vec (its `Drop`
  aborts the console `sub_task`, per the existing no-leak design). Use a small
  handle (index or an `Rc`-tracked id) to locate the entry.

`Console` component itself is unchanged (already builds its VTE terminal
imperatively and aborts its task on Drop).

## Data flow

Bridge setup → `net-up` → bridge has `192.168.100.1`, dnsmasq leases `.2+`,
masquerade routes guest traffic out the default iface → guest boots, DHCP lease,
internet. VM launch → ch stdout/stderr appended to `chimera.log`. Console button
→ new window with its own VTE, several coexist.

## Error handling

- `net-up` is idempotent (addr flush, `nft add table` no-ops if present,
  iptables `-C` guards); safe to re-run when re-creating the bridge.
- `net-down` tolerates missing table/rules/pidfile (teardown never hard-fails).
- Missing dnsmasq: `net-up`'s dnsmasq spawn fails → surfaced as an error;
  `doctor` flags it up front so the user installs it first.
- ch-log open failure falls back to inherited stdio; never blocks spawn.
- Console window close removing a not-found controller is a no-op.

## Testing

Unit (default CI, no root):
- `netops`: `net_up_cmds` (nft + iptables variants: addr, forward, masquerade,
  order, `chimera_nat` table name, `<cidr>`), `net_down_cmds` (delete/flush +
  addr flush), `dnsmasq_cmd` (interface/bind/dhcp-range/pidfile flags).
- `setup`: `net_up_argv`/`net_down_argv` (absolute netd path, subnet constants,
  pidfile path); NAT constants sane (`.1` in `/24`).
- `supervisor`: with a `log_path`, spawned child's stdout is redirected to the
  file (spawn a wrapper script that echoes; assert the file contains the line).

Manual: create the bridge in the GUI; `ip addr show chibr0` shows
`192.168.100.1`; boot a VM; guest gets a `192.168.100.x` lease and reaches the
internet; `chimera.log` contains ch `<vmm>` lines; open two console windows at
once.

## Out of scope (deferred)

- IPv6 NAT / DHCPv6.
- Multiple/custom subnets or per-VM networks (single fixed NAT network).
- Persisting `ip_forward` across reboots (runtime only).
- Port-forwarding / inbound rules to guests.
- Firewalld/ufw integration (raw nft/iptables only).
