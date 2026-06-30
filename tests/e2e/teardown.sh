#!/usr/bin/env bash
# Reverse setup.sh. Idempotent — safe to run repeatedly. Run as root (sudo).
set -uo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BRIDGE="${CHIMERA_TEST_BRIDGE:-chibr0}"
RULE="/etc/polkit-1/rules.d/49-chimera-netd-test.rules"

[ "$(id -u)" -eq 0 ] || { echo "teardown.sh must run as root (use sudo)" >&2; exit 1; }

ip link del "$BRIDGE" 2>/dev/null || true
rm -f "$RULE"
rm -f /usr/libexec/chimera-netd
rm -f /usr/share/polkit-1/actions/org.chimera.netd.policy
rm -f "$HERE/env.sh"

echo "teardown complete (bridge $BRIDGE, polkit rule, helper, policy, env.sh removed)"
