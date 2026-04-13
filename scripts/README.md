# scripts/

Helpers for the two-machine development workflow.

openclank is developed on macOS but must run on illumos.
These scripts bridge the gap — syncing code and credentials
to the OmniOS test box so that `cargo test` and live API
tests can run on the target platform. The
[`Makefile`](../Makefile) wraps them into `make` targets.
