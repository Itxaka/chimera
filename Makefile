.PHONY: e2e-setup e2e e2e-teardown e2e-all

# Provision the host (root): netd + polkit rule + bridge + firmware.
e2e-setup:
	sudo tests/e2e/setup.sh

# Run the gated e2e suite (requires e2e-setup first).
e2e:
	. tests/e2e/env.sh && cargo test -p chimera-core -- --ignored --nocapture

# Reverse provisioning (root).
e2e-teardown:
	sudo tests/e2e/teardown.sh

# Full cycle: provision, run (always tear down even on failure).
e2e-all:
	sudo tests/e2e/setup.sh
	-. tests/e2e/env.sh && cargo test -p chimera-core -- --ignored --nocapture
	sudo tests/e2e/teardown.sh
