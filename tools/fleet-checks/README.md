# tools/fleet-checks

The PH-12 fleet checks as approved named checks for phlow seats:
`theodolite_zero.py`, `toolchain_pin.py`, `tree_freeze.py`.
Each is a standalone Python 3 program (stdlib only) with a
documented contract in its module docstring: exit 0 pass, 1 the
gate's condition is false, 2 unproven.

The document of record — the torogao seat (PH-10), the check
contracts, and the recorded gate evidence — is
`docs/FLEET_CHECKS.md`. Behaviour is pinned by
`crates/phlow-checks/tests/fleet_checks.rs`.
