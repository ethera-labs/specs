# Privacy

All transaction data is saved on the operator's private server. We will analyze in the next section how the processes in each sub-protocol ensure data privacy doesn't break.


## Settlement

Sequencers provide to the SP, along with a proof, the following _public/committed value_ that the ZK program outputs:

```rust
struct AggregationOutputs {
    bytes32 l1Head;           // = AggregationInputs.latest_l1_checkpoint_head
    bytes32 l2PreRoot;        // = boot_infos[0].l2PreRoot (pre-root of the first range)
    bytes32 l2PostRoot;       // = boot_infos[last].l2PostRoot (post-root of the last range)
    uint64 l2BlockNumber;     // = boot_infos[last].l2BlockNumber (last L2 block number)
    bytes32 rollupConfigHash;
    bytes32 multiBlockVKey;   // Range program verification key
    address proverAddress;    // Prover address

    bytes32 mailboxRoot;      // (NEW) = boot_infos[last].mailboxRoot (mailbox root of the last range)
}
```

Besides this, it sends to the SP a list of `(chain ID, inbox root, outbox root)` with the updated mailbox roots for each chain.

Because the SP only receives commitments data, and not the actual L2 transactions, it cannot infer any information about the transactions included in the rollup. The SP can only verify that the commitments are correct based on the proofs provided by the sequencers.


## Superblock Construction Protocol (SBCP)

The following messages are exchanged:

```proto
// SP -> Sequencers
message StartPeriod {
  uint64 period_id = 1;
  uint64 superblock_number = 2;
}
// SP -> Sequencers
message Rollback {
  uint64 period_id = 1;
  uint64 last_finalized_superblock_number = 2;
  bytes last_finalized_superblock_hash = 3;
}

// Sequencers -> SP
message Proof {
  uint64 period_id = 1;
  uint64 superblock_number = 2;
  bytes proof_data = 3;
}
```

The only possible concern could be the `proof_data` field, but is defined by the settlement.
As we reviewed in the settlement section, it only contains commitments data, without any sensitive information.


## Synchronous Composability Protocol (SCP)

The SCP is responsible for allowing sequencers and SP to agree on the inclusion of cross-chain transactions. The messages exchanged after the cross-chain transaction request are:

```proto
// Sequencer -> SP
message Vote {
  uint64 transaction_id = 1;
  bool vote = 2; // true = commit, false = abort
  bytes commitment = 3; // Optional: Commitment or identifier of the transaction
}

// SP -> Sequencer
message Decided {
  uint64 transaction_id = 1;
  bool decided = 2; // true = commit, false = abort
  bytes justification = 3; // Optional: (could include proofs, aggregated votes, etc.)
}
```

By sending only transaction ids to the shared publisher we keep the data confidential.

