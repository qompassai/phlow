# Agent skills for phlow

This directory holds vendored agent skills for working on the phlow repo —
finishing the Rust build-out and publishing the crates. They are copies of
Matt's canonical skills, vendored here so any agent (Claude Code or opencode)
working in this checkout gets the same rules without extra setup.

Skills: `tiger-style-rust` (all Rust code), `tiger-style-nix` (flake.nix /
dev shells), `tiger-style-mojo` (Mojo kernels and worker), `git-wip-guard`
(never destroy uncommitted work), `phlow-publish` (crates.io release
checklist: gates, version audit, topological publish order, tagging).
