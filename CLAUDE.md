# MPFree: notes for AI agents

Read [docs/agents/README.md](docs/agents/README.md) before changing anything. In short:

- **Safety first.** Nothing that touches the MPC One is given to the owner unless it is verified correct and reversible, labelled read-only or state-changing, with its undo. No writes to the eMMC boot partitions, U-Boot environment or root filesystem.
- **Docs live in three places:** `docs/user/` (the user manual, for musicians), `docs/dev/` (developers), `docs/agents/` (agents). Put new pages there and list them in that folder's index.
- **Docs change in the same pull request as the code.** A change to what a musician sees or does updates its manual page; a change to an engine updates its design page. See [docs/agents/docs-system.md](docs/agents/docs-system.md).
- **The shared screen layout and the hardware buttons follow the approved [layout spec](docs/dev/design/layout-spec.md).** Propose changes to its owner; don't change them yourself.
- **Before a pull request:** `cargo test`, `cargo fmt --check` and `cargo clippy` on the workspace (exact commands in docs/agents/README.md), plus `python3 docs/tools/check_docs.py`.
- **Code pull requests merge only on the owner's go.**
