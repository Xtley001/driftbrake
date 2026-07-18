# Security Policy

`driftbrake` sits directly in the decision path of a live trading strategy — a bug in `reconcile` or `revm-backend` has direct capital consequences, either by failing to halt a strategy that should have stopped, or by producing an incorrect simulation result the strategy trusts. Security issues here are taken as seriously as in any code that touches funds directly, even though `driftbrake` itself never holds custody of assets.

Before reporting, it's worth reading the [whitepaper](https://xtley001.github.io/driftbrake/whitepaper.html#6-security-considerations) — Section 6 (Security Considerations) already documents the specific failure modes this project treats as security-relevant, which is a useful check against duplicate reports for known, already-documented tradeoffs.

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x (pre-1.0) | Yes, while pre-1.0 |
| < 0.1.0 (pre-release / unpublished) | No |

Until a 1.0 release, only the latest published `0.x` minor version receives security fixes. Pin a specific version in production and review the changelog before upgrading rather than tracking `main`.

## What counts as a security issue here

Not just memory safety or classic CVE-shaped bugs — for this crate, the following are all in scope:

- A `HaltPolicy` implementation (default or otherwise) that fails to halt under conditions it is documented to catch (a false negative in the fast or slow guard).
- The ratio-direction invariant (Property 1 in [`docs/whitepaper.md`](./docs/whitepaper.md#formal-properties-and-invariants)) being violated in any code path, including edge cases around zero, negative, or missing predicted profit.
- A condition under which an adversary (e.g. a malicious or compromised venue) could construct transactions that reliably evade both guards while still degrading realized profit — i.e., a gap in guard coverage that isn't just a threshold-tuning question but a structural blind spot.
- `revm-backend` returning a materially incorrect `RawSimOutput` (predicted profit, gas estimate, or revert status) under conditions that are plausible in production, not just adversarial edge cases.
- `receipt-poller` crediting profit from a receipt that should not have counted (e.g. a reverted transaction, or a log from an unrelated contract) — see the receipt-gated realization discussion in the whitepaper's Section 4.5.
- Any panic, unbounded resource growth, or deadlock reachable from processing attacker-influenceable input (a malicious RPC response, a crafted receipt, adversarial mempool data if a future version consumes it).

Standard supply-chain concerns (dependency vulnerabilities, unsound `unsafe` blocks) are also in scope but are a smaller share of the actual risk surface for this crate compared to the logic issues above.

## Reporting a vulnerability

**Do not open a public GitHub issue for a security report.** Public issues are appropriate for functional bugs (see [`CONTRIBUTING.md`](./CONTRIBUTING.md)) but not for anything that could give an adversary a working exploit before a fix ships.

Instead:

1. Use GitHub's private vulnerability reporting feature on this repository (Security tab → "Report a vulnerability"), or email the maintainers directly if that feature is unavailable — see the repository's contact information for the current address.
2. Include: which crate is affected (`core`, `revm-backend`, `reconcile`, `receipt-poller`, `telemetry`), a minimal reproduction if possible, and your assessment of impact (does this cause a missed halt, a false halt, an incorrect profit figure, or something else).
3. If the issue involves a specific chain, RPC provider, or ABI shape that triggered it, include that context — several of the failure modes this crate exists to prevent are chain- or timing-dependent and don't reproduce identically everywhere.

## What to expect

- Acknowledgment of a report within a reasonable timeframe.
- An assessment of severity and, for confirmed issues, a target timeline for a fix communicated back to the reporter.
- Credit in the fix's changelog entry and release notes, unless you prefer to remain anonymous — state your preference in the initial report.
- Coordinated disclosure: we ask that reporters hold off on public disclosure until a fix is released and users have had a reasonable window to upgrade, given that this crate gates live capital decisions for anyone running it.

## Scope boundaries

This policy covers the `driftbrake` crate itself. It does not cover:

- Vulnerabilities in a downstream strategy's own `ProfitDecoder` or `RealizedProfitDecoder` implementation — those are the implementer's responsibility, though we're happy to advise if you believe the trait contract itself makes a correct implementation unreasonably difficult.
- The `toy-arbitrage` example, which is illustrative code, not production-hardened — do not run it against real capital without your own review.
- Third-party RPC providers, node software, or infrastructure the crate connects to but does not control.
