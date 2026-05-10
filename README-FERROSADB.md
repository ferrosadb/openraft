# ferrosadb/openraft — Ferrosa's openraft fork

This is a fork of [databendlabs/openraft](https://github.com/databendlabs/openraft)
maintained by the [Ferrosa](https://github.com/ferrosadb/ferrosa) project.
It tracks the upstream `release-0.9.x` line and carries patches that Ferrosa
needs but that are either declined upstream or not yet merged.

If you are an end user of openraft, **use upstream** — this fork is for
Ferrosa's consumption.

## Why we forked

Three protocol-level capabilities are required by Ferrosa for production
correctness and operability, and are not in upstream openraft 0.9.x:

### 1. PreVote (Ongaro §9.6)

**Status upstream**: declined. The openraft author has [explicitly stated](https://github.com/databendlabs/openraft/discussions/15)
that PreVote is not a goal — the upstream substitute is the leader-lease
state machine combined with `Vote` ordering.

**Why Ferrosa needs it**: the lease substitute does not cover all the
liveness scenarios PreVote is designed for. Specifically, after a
partition heal where every follower's lease has lapsed, a rejoining node
with an inflated `(committed, term, voted_for)` triple can still poison
the cluster. CockroachDB hit exactly this (cockroach#92088) and added
both PreVote and CheckQuorum (PR #104042). The decentralizedthoughts.github.io
proof from 2020-12-12 states: **without both, Raft does not guarantee
liveness under network omission faults**.

Ferrosa observed an unrecoverable runaway-term failure in production
(node3 ran term to T18,348 versus a leader at T8 over 32 hours), tracked
as `bug-raft-stale-candidate-runaway-term-no-prevote.md`. PreVote in
this fork makes that scenario non-reproducible.

### 2. CheckQuorum (Ongaro §6.4)

**Status upstream**: not implemented. Upstream tracks lease state but
the leader does not voluntarily step down when its lease lapses without
a quorum-ack — it stays in zombie-leader mode and clients hang.

**Why Ferrosa needs it**: under asymmetric or partial cross-DC
partitions, the zombie-leader window blocks all writes against the
affected DC until something else (failure detection, timeouts, manual
intervention) resolves the situation. Ferrosa's CheckQuorum default
ratio is `0.75` (configurable via `FERROSA_RAFT_CHECK_QUORUM_RATIO`)
chosen for Ferrosa's longer election timeouts (3000–6000 ms vs upstream
defaults around 150–300 ms) so the zombie window is bounded at ~2.25–4.5
seconds rather than the upstream 6 seconds.

### 3. Leadership Transfer (Ongaro §3.10)

**Status upstream**: not exposed. `Raft::trigger()` provides
`elect()` / `heartbeat()` / `snapshot()` / `purge_log()` but no targeted
`transfer_to(node_id)` with TimeoutNow semantics.

**Why Ferrosa needs it**: graceful drains for DC-aware operations
(decommissioning a leader before draining its DC). Without this,
operator-initiated leader handover races against the natural election
timer.

### Pre-existing patch already on this fork

The branch `fix/separate-replication-timeout` (already merged into
`main`) splits `replication_lag_timeout` from
`heartbeat_interval`. Upstream conflates the two; under sled disk-IO
contention this caused perpetual replication timeouts in Ferrosa.

## What's in this fork that's NOT in upstream

| Patch | Files | Status with upstream |
|---|---|---|
| `fix/separate-replication-timeout` | `engine/`, `core/`, `network/` | Filed-eligible; not yet submitted |
| PreVote (W3.1–W3.4) | `engine/pre_vote_decision.rs`, `raft/message/pre_vote.rs`, `engine/leader_lease.rs`, `engine/handler/vote_handler/`, etc. | Declined upstream; carry as fork-only |
| CheckQuorum (W3.5–W3.7) | `engine/check_quorum.rs`, `engine/engine_impl.rs` (tick handler), `engine/command.rs` | Filed-eligible; not yet submitted |
| Leadership Transfer (W3.8–W3.10) | `raft/message/timeout_now.rs`, `raft/trigger.rs`, `engine/handler/timeout_now/`, integration tests | Filed-eligible; not yet submitted |

Total surface: ~2300 added lines across `openraft/src/` and `tests/`,
zero changes to `macros/` or other workspace crates.

## Maintenance commitment

We maintain this fork for the duration of Ferrosa's production lifetime.
Concretely:

1. **Track upstream `release-0.9.x`**. When upstream cuts a new patch
   release on the 0.9 line, we rebase our patches onto it within ≤ 2
   weeks and re-publish under the next ferrosadb build-metadata tag.

2. **Evaluate openraft 1.0 migration when stable**. ADR-018 in the
   Ferrosa repo lays out the criteria. Until then, we stay on 0.9.x.

3. **CI**: every push to `main` runs the full
   upstream test suite (`cargo test`) plus our additional tests
   (`tests/tests/elect/t12_pre_vote_basic.rs`,
   `tests/tests/membership/t90_transfer_leader.rs`,
   `engine/tests/handle_pre_vote_req_test.rs`,
   `engine/tests/handle_timeout_now_test.rs`). Failures block merges.

4. **No upstream PRs from this fork** — by design. We track upstream;
   we do not solicit upstream changes from here. If the upstream
   maintainers want any of these patches, they can adopt them
   independently. This decision is final and intentional, not a
   placeholder.

5. **Versioning**: `0.9.25+ferrosadb.N` denotes the Nth ferrosadb
   patch series on top of upstream `0.9.24` (the latest upstream
   `0.9.x` release). The numeric prefix is chosen to satisfy `^0.9`
   requirements while remaining greater than upstream; the
   `+ferrosadb.N` build-metadata identifies the fork without affecting
   semver precedence. When we rebase onto a future upstream `0.9.x`
   release the numeric prefix advances to stay one ahead of upstream
   while remaining in the `0.9` line.

6. **Public**: this fork is open-source under the same dual MIT /
   Apache-2.0 license as upstream. Other consumers can use it; we do
   not gate access.

## Branches

- `main` — **production branch for Ferrosa**; carries all patches on
  top of upstream `release-0.9.x`. This is what `ferrosa-suite`
  Cargo.toml's `[patch.crates-io]` points at, and the default branch
  of this repo.
- `upstream-0.10-mirror` — read-only mirror of upstream `databendlabs/
  openraft` `main` (the 0.10 line). Kept for reference; not used by
  Ferrosa, which stays on 0.9.x per ADR-018.
- `fix/separate-replication-timeout` — historical patch branch, now
  rolled into `main`. Kept for reference.

## Configuration knobs added by this fork

| Env var | Default | Purpose |
|---|---|---|
| `FERROSA_RAFT_CHECK_QUORUM_RATIO` | `0.75` | CheckQuorum step-down threshold as multiple of `election_timeout`. Lower = faster step-down on partition; higher = more tolerant of transient blips. |
| (Cargo feature) `enable_pre_vote` | `true` in Ferrosa builds | PreVote round before term advance. Disabling restores upstream behaviour. |

## Upstream README

The upstream openraft README lives at [README.md](README.md) and
documents the project's design, usage, and contribution guidelines. We
do not modify it; refer to it for everything not specific to this fork.
