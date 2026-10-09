# Security Policy

`driftbrake` operates directly within algorithmic execution and risk management loops. A failure in simulation or reconciliation can cause direct capital loss. Security reports are treated with the highest priority.

For documented trade-offs and structural risk vectors, review Section 6 of the [whitepaper](./docs/whitepaper.md#6-security-considerations) prior to submission.

## Supported Versions

| Version | Status | Notes |
|---|---|---|
| `0.2.x` | Supported | Current release line (Rust & Python) |
| `0.1.x` | Supported | Previous release line |
| `< 0.1.0` | Unsupported | Pre-release and unreleased commits |

## Scope

The following failure modes are considered security-critical:

- **Halt policy failures**: a guard failing to trip under conditions it is specified to catch.
- **Ratio-direction inversion**: violation of Property 1 (computing $\hat{p}_i / r_i$ instead of $r_i / \hat{p}_i$).
- **Adversarial evasion**: transaction sequences engineered to evade fast and slow guards while systematically draining capital.
- **Simulation inaccuracies**: `revm-backend` returning invalid execution output or gas estimates under production-plausible conditions.
- **Spurious profit realization**: `receipt-poller` crediting profit from reverted transactions or unrelated logs.
- **State corruption**: `driftbrake-journal` accepting corrupted records or failing to truncate torn writes cleanly.
- **Denial of service**: panics, unconstrained memory allocation, or runtime deadlocks triggered by adversarial RPC responses or receipts.

## Reporting a Vulnerability

Do not open public GitHub issues for security vulnerabilities.

1. Submit a private report via **GitHub Security Advisories** (navigate to Security → Advisories → "Report a vulnerability").
2. Provide details including:
   - Affected crate (`core`, `reconcile`, `journal`, `receipt-poller`, `revm-backend`, `driftbrake-py`).
   - Minimal reproduction code or transaction receipts.
   - Severity assessment (missed halt, spurious halt, or state corruption).
   - Target network, RPC provider, or execution context.

## Coordinated Disclosure

- Maintainers acknowledge reports within 48 hours.
- Confirmed issues receive a severity rating and remediation timeline.
- Fixes are published via coordinated minor releases. Reporters receive attribution unless anonymity is requested.

## Exclusions

This policy covers the `driftbrake` workspace crates. The following are out of scope:

- Downstream implementations of `ProfitDecoder` or `RealizedProfitDecoder`.
- Illustrative examples (`examples/toy-arbitrage`).
- External RPC infrastructure, node software, or RPC provider rate limits.
