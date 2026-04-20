# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check

# Run a single test by name
cargo test -p ethera-spec-scp test_name

# Run all tests in a specific crate
cargo test -p ethera-spec-sbcp

# Full verification (fmt + clippy + test + build)
make verify
```

The workspace root is at the repo root. Toolchain is pinned to stable 1.82 via `rust-toolchain.toml`.

## Architecture

This is the canonical specification library for the Compose protocol – a network of rollups with synchronous, atomic composability through a shared publisher. The Rust crates are designed to be imported by downstream rollup implementations (sequencers, publishers, provers).

### Crate Dependency Graph

```text
ethera-spec          (core types: ChainId, InstanceId, XtRequest, etc.)
  |
  +-- ethera-spec-scp    (SCP: 2-phase commit for a single cross-chain tx)
  +-- ethera-spec-sbcp   (SBCP: orchestrates periods, blocks, proof aggregation)
  +-- ethera-spec-proto  (protobuf wire format via prost)
```

### Two-Layer Protocol

**SCP (Synchronous Composability Protocol)** -- `crates/scp/`

- Implements 2PC (two-phase commit) for a single cross-chain transaction.
- `PublisherInstance`: coordinator that collects votes from sequencers, decides accept/reject.
- `SequencerInstance`: participant that simulates transactions, handles mailbox read/write for cross-chain data, votes.
- Both sides are parameterized by network/execution traits (`PublisherNetwork`, `SequencerNetwork`, `ExecutionEngine`) that consumers implement.

**SBCP (Superblock Construction Protocol)** -- `crates/sbcp/`

- Orchestration layer on top of SCP: manages periods, superblock numbering, block construction, settlement pipeline.
- `Publisher`: coordinates periods, starts SCP instances, aggregates ZK proofs from all chains, publishes to L1.
- `Sequencer`: manages block lifecycle (begin/end), instance locking (local tx exclusion during SCP), triggers proof generation and settlement.
- Both parameterized by prover/messenger/L1 traits.

### Concurrency Pattern

All stateful types use `Mutex<InnerState>` with the lock held for short critical sections. The trait interfaces (`PublisherNetwork`, `ExecutionEngine`, etc.) are `Send + Sync` to support async runtimes in consumers, but the spec library itself is synchronous.

### Proto Crate

`crates/proto/` contains hand-written prost structs (not code-generated from `.proto` files). The `convert.rs` module provides `From`/`TryFrom` conversions between proto wire types and domain types from `ethera-spec`.

## Lint Configuration

- Clippy runs with `deny(clippy::all)` and `warn(clippy::pedantic)` at workspace level.
- `#[must_use]` is annotated on pure query methods. `unused_must_use` is denied.
- Pre-commit hooks enforce `cargo fmt` and `cargo clippy` on every commit.
