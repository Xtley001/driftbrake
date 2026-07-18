# Contributing to driftbrake

Thanks for considering a contribution. This project is intentionally small in scope (see [Non-goals](./docs/ARCHITECTURE.md#non-goals)) — the fastest way to get a PR merged is to work within that scope rather than expand it.

## Development setup

```bash
git clone https://github.com/Xtley001/driftbrake.git
cd driftbrake
rustup show                # confirm the toolchain version matches rust-toolchain.toml
cargo build --workspace
cargo test --workspace
```

Several `Cargo.toml` files (`revm-backend`, `driftbrake`,
`examples/toy-arbitrage`) carry explicit `"=x.y.z"` version pins on a
handful of transitive dependencies. These exist to route around an old,
network-restricted build environment used during initial development and
are not a deliberate compatibility decision — on a normal, current
toolchain with full network access, run `cargo update`, then remove the
pins (`grep -n '"=' */Cargo.toml examples/*/Cargo.toml` finds all of
them) and run `cargo update` once more.

### Requirements

- Rust, pinned version per `rust-toolchain.toml` in the repo root.
- An RPC endpoint with archive-node access if you're working on `revm-backend` or running the `toy-arbitrage` example against a fork — a free-tier public RPC is usually sufficient for local iteration but will rate-limit under the bounded-concurrency benchmark load.

## Workspace layout

The crate is a Cargo workspace with a hard boundary between `core` and everything else — see [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for the full module map and rationale.

```
driftbrake/      # facade crate: cargo add driftbrake re-exports core + reconcile + receipt-poller
core/            # trait definitions only — no REVM, no alloy, no RPC client types (crate: driftbrake-core)
revm-backend/    # the only crate allowed to depend on REVM directly (crate: driftbrake-revm-backend) —
                 # separate/opt-in, not bundled into the driftbrake facade, see docs/ARCHITECTURE.md
reconcile/       # the default HaltPolicy implementation (phantom-guard) (crate: driftbrake-reconcile)
receipt-poller/  # receipt confirmation + realized-profit decoding (crate: driftbrake-receipt-poller)
telemetry/       # planned, not yet built — structured event emission for the benchmark harness
benchmark/       # docs/BENCHMARK.md's methodology as a runnable tool (crate: driftbrake-benchmark)
examples/        # end-to-end examples, not published to crates.io
```

**If your change adds a dependency to `core`, that's a signal to stop and reconsider the change**, not a signal to add an `#[cfg(feature = ...)]` workaround. `core`'s zero-dependency property is load-bearing (it's what makes `revm-backend` swappable) and PRs that compromise it will be asked to restructure before merge.

## Where a given change belongs

| You're changing... | It belongs in... |
|---|---|
| A trait signature (`ProfitDecoder`, `RealizedProfitDecoder`, `HaltPolicy`) | `core` — expect extra scrutiny, this is the public contract everything else depends on |
| REVM forking, simulation timeout logic, concurrency handling | `revm-backend` |
| Guard math, threshold defaults, halt decision logic | `reconcile` |
| Receipt polling, confirmation logic, gas netting | `receipt-poller` |
| A new structured event type | `telemetry` |
| A new end-to-end demo strategy | `examples/` |

## Before opening a PR

- **Run the full test suite**, not just the crate you touched: `cargo test --workspace`. Changes to `core` trait signatures ripple into every downstream crate.
- **If you touched `reconcile`, run the ratio-direction regression test explicitly** and confirm it still passes: `cargo test -p driftbrake-reconcile ratio_direction`. This test exists specifically to catch the inversion bug described in [`docs/whitepaper.md`](./docs/whitepaper.md#formal-properties-and-invariants) (Property 1) — a passing test suite that happens to skip this one is not sufficient.
- **If you changed a default threshold or window size**, re-run the benchmark sweep (see [`docs/BENCHMARK.md`](./docs/BENCHMARK.md)) and include the resulting false-halt/missed-catch curve in your PR description. A threshold change without benchmark evidence will be asked for it before review.
- **Run `cargo fmt` and `cargo clippy --workspace --all-targets`** — both are checked in CI and a red CI run will block merge regardless of the change's substance.
- **Update `CHANGELOG.md`** under the `Unreleased` section, in the appropriate `Added` / `Changed` / `Fixed` subgroup.

## Commit and PR conventions

- Keep commits scoped to one logical change; a PR that touches `core`, `reconcile`, and an example together is harder to review and more likely to be asked to split.
- PR description should state which crate(s) are affected and why, using the table above as a guide if it's not obvious.
- Reference the relevant section of `docs/ARCHITECTURE.md` or `docs/whitepaper.md` if your change affects documented behavior — reviewers will check the docs are updated in the same PR, not left stale.

## What we will not accept

Per the documented [non-goals](./docs/ARCHITECTURE.md#non-goals), PRs adding the following will be declined regardless of quality, and are better served as a separate crate that depends on `driftbrake`:

- Alerting integrations (Telegram, PagerDuty, Discord webhooks, etc.)
- Per-strategy EOA/nonce management
- USD-denominated gas budgeting or any price-oracle dependency inside `core` or `revm-backend`
- Inventory-aware unwind logic (tracked as a deliberate v1 non-goal, not an oversight — see `docs/ARCHITECTURE.md`)

If you have a use case that needs one of these, we're happy to discuss how to build it as a separate crate that consumes `driftbrake`'s `telemetry` events rather than folding it into core scope — open an issue to discuss before investing in a PR.

## Publishing a release

Maintainer-only. `cargo publish` requires every path dependency to also
carry a `version`, which is already set up — but publishing order still
matters, since each crate depends on the previous one actually being live
on the registry:

```bash
cargo publish -p driftbrake-core
# wait for it to be indexed before the next one
cargo publish -p driftbrake-reconcile
cargo publish -p driftbrake-revm-backend
cargo publish -p driftbrake-receipt-poller
cargo publish -p driftbrake
cargo publish -p driftbrake-benchmark
```

`examples/toy-arbitrage` has `publish = false` and is skipped
automatically. Sanity-check each crate with `cargo publish -p <name>
--dry-run` before the real thing — it packages and verifies without
uploading. docs.rs builds and hosts documentation automatically within a
few minutes of each publish; nothing to configure.

## Design docs site

Live at **[xtley001.github.io/driftbrake](https://xtley001.github.io/driftbrake/)**.
`docs/*.md` is built into a browsable site via
[mdBook](https://rust-lang.github.io/mdBook/) (config: `book.toml`, theme:
`theme/custom.css`) and deployed automatically by
`.github/workflows/docs.yml` on every push to `main` that touches
`docs/`, `theme/`, or `book.toml`.

**One-time setup, not automated by the workflow itself:** in the repo's
GitHub Settings → Pages, set the source to **GitHub Actions**. Until
that's set once, the workflow will build successfully but the deploy
step has nowhere to publish to.

To preview locally before pushing:

```bash
cargo install mdbook --locked
mdbook serve --open
```

## Reporting bugs vs. security issues

Regular bugs: open a GitHub issue with a minimal reproduction. For anything that could cause a live strategy to fail to halt when it should (a false negative in `reconcile`), or halt spuriously in a way that could be exploited (a false positive triggerable by an adversary), follow [`SECURITY.md`](./SECURITY.md) instead of a public issue.
