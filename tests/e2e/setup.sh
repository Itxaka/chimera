#!/usr/bin/env bash
# Provision the host for Chimera e2e tests. Run as root (sudo).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
BRIDGE="${CHIMERA_TEST_BRIDGE:-chibr0}"
USER_NAME="${CHIMERA_TEST_USER:-${SUDO_USER:-$USER}}"
FW_CACHE="${CHIMERA_TEST_FW_CACHE:-/var/cache/chimera-e2e}"
FW_VERSION="${CHIMERA_FW_VERSION:-0.4.2}"
RULE="/etc/polkit-1/rules.d/49-chimera-netd-test.rules"
POLICY_SRC="$REPO/packaging/org.chimera.netd.policy"
ENV_OUT="$HERE/env.sh"

[ "$(id -u)" -eq 0 ] || { echo "setup.sh must run as root (use sudo)" >&2; exit 1; }

# Preflight — fail fast with a clear message.
[ -e /dev/kvm ] || { echo "FATAL: /dev/kvm not present" >&2; exit 1; }
command -v cloud-hypervisor >/dev/null || { echo "FATAL: cloud-hypervisor not on PATH" >&2; exit 1; }
command -v ip >/dev/null || { echo "FATAL: ip (iproute2) not found" >&2; exit 1; }
command -v pkexec >/dev/null || { echo "FATAL: pkexec (polkit) not found" >&2; exit 1; }

# Build + install the privileged helper and its polkit policy.
( cd "$REPO" && cargo build -p chimera-netd --release )
install -Dm0755 "$REPO/target/release/chimera-netd" /usr/libexec/chimera-netd
install -Dm0644 "$POLICY_SRC" /usr/share/polkit-1/actions/org.chimera.netd.policy

# Passwordless polkit rule for the test user only.
cat > "$RULE" <<EOF
// Installed by chimera tests/e2e/setup.sh — allows the test user to run the
// chimera-netd polkit action without a prompt. Removed by teardown.sh.
polkit.addRule(function(action, subject) {
    if (action.id == "org.chimera.netd.manage" && subject.user == "$USER_NAME") {
        return polkit.Result.YES;
    }
});
EOF

# Throwaway bridge.
if ! ip link show "$BRIDGE" >/dev/null 2>&1; then
    ip link add name "$BRIDGE" type bridge
fi
ip link set "$BRIDGE" up

# Firmware: use CHIMERA_TEST_FW if provided, else fetch a pinned blob.
if [ -n "${CHIMERA_TEST_FW:-}" ]; then
    FW="$CHIMERA_TEST_FW"
else
    mkdir -p "$FW_CACHE"
    FW="$FW_CACHE/hypervisor-fw"
    if [ ! -f "$FW" ]; then
        echo "fetching rust-hypervisor-firmware $FW_VERSION ..."
        curl -fL -o "$FW" \
          "https://github.com/cloud-hypervisor/rust-hypervisor-firmware/releases/download/$FW_VERSION/hypervisor-fw"
    fi
fi
[ -f "$FW" ] || { echo "FATAL: firmware not found at $FW" >&2; exit 1; }

# Emit env for the test run (owned by the test user).
cat > "$ENV_OUT" <<EOF
export CHIMERA_E2E=1
export CHIMERA_TEST_BRIDGE="$BRIDGE"
export CHIMERA_TEST_FW="$FW"
EOF
chown "$USER_NAME" "$ENV_OUT" 2>/dev/null || true

echo "setup complete. bridge=$BRIDGE user=$USER_NAME fw=$FW"
echo "run: make e2e   (or: source tests/e2e/env.sh && cargo test -p chimera-core -- --ignored)"
