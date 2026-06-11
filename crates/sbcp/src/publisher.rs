use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use ethera_spec::{
    chains_from_request, ChainId, DecisionState, Instance, InstanceId, PeriodId, SequenceNumber,
    SuperblockHash, SuperblockNumber, XtRequest,
};
use ethera_spec_scp::{PublisherInstance, PublisherNetwork};
use thiserror::Error;
use tracing::{error, info, warn};

use crate::id::generate_instance_id;

/// Errors returned by [`Publisher`] operations.
#[derive(Debug, Error)]
pub enum PublisherError {
    #[error("can not start any instance")]
    CannotStartInstance,
    #[error("can not start period: target superblock is {target}, expected {expected}")]
    CannotStartPeriod { target: u64, expected: u64 },
    #[error("can not advance to older settled state")]
    OldSettledState,
    #[error("invalid request")]
    InvalidRequest,
    #[error("target superblock is less than the last finalized one")]
    InvalidInitialState,
    #[error("unknown instance")]
    UnknownInstance,
    #[error(transparent)]
    Scp(#[from] ethera_spec_scp::PublisherError),
    #[error("proof from a chain that is not part of the network")]
    ProofFromUnknownChain,
    #[error("proof for already finalized superblock")]
    ProofForFinalizedSuperblock,
    #[error("proof for non-terminated superblock")]
    ProofForNonTerminatedSuperblock,
    #[error("proof for superblock that is not the next to settle")]
    ProofNotSequential,
    #[error("proof for wrong period: expected {expected}, received {received}")]
    ProofWrongPeriod { expected: u64, received: u64 },
    #[error("duplicate proof for chain")]
    DuplicateProof,
    #[error("superblock proof generation failed: {0}")]
    ProverFailed(String),
}

/// Outcome of a successfully accepted proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofStatus {
    /// Accepted; more chains still need to report.
    Collected { received: usize, required: usize },
    /// All chains reported; the superblock proof was generated and published.
    Published,
}

/// Generates and aggregates ZK proofs for a superblock.
pub trait PublisherProver: Send + Sync {
    /// Per-chain aggregation proof received from a sequencer.
    type ChainProof: Send;
    /// Superblock network proof published to L1.
    type SuperblockProof: Send;

    fn request_superblock_proof(
        &self,
        superblock_number: SuperblockNumber,
        last_superblock_hash: SuperblockHash,
        proofs: HashMap<ChainId, Self::ChainProof>,
    ) -> Result<Self::SuperblockProof, Box<dyn std::error::Error + Send + Sync>>;
}

/// Broadcasts protocol messages to connected sequencers.
pub trait PublisherMessenger: Send + Sync {
    fn broadcast_start_period(
        &self,
        period_id: PeriodId,
        target_superblock_number: SuperblockNumber,
    );
    fn broadcast_rollback(
        &self,
        period_id: PeriodId,
        superblock_number: SuperblockNumber,
        superblock_hash: SuperblockHash,
    );
}

/// Publishes proofs to L1.
pub trait L1Publisher: Send + Sync {
    /// Superblock network proof published to L1.
    type SuperblockProof: Send;

    fn publish_proof(&self, superblock_number: SuperblockNumber, proof: Self::SuperblockProof);
}

struct InstanceEntry<N: PublisherNetwork> {
    machine: Arc<PublisherInstance<N>>,
    chains: Vec<ChainId>,
}

struct PublisherState<Proof, N: PublisherNetwork> {
    period_id: PeriodId,
    target_superblock_number: SuperblockNumber,
    last_finalized_superblock_number: SuperblockNumber,
    last_finalized_superblock_hash: SuperblockHash,
    proofs: HashMap<SuperblockNumber, HashMap<ChainId, Proof>>,
    chains: HashSet<ChainId>,
    sequence_number: SequenceNumber,
    active_chains: HashSet<ChainId>,
    instances: HashMap<InstanceId, InstanceEntry<N>>,
    proof_window: u64,
}

/// SBCP publisher coordinator managing periods, SCP instances, and proof
/// aggregation.
///
/// Owns one [`PublisherInstance`] per in-flight cross-chain transaction:
/// [`Publisher::start_instance`] creates and launches it, votes are routed via
/// [`Publisher::process_vote`], and stale instances are aborted via
/// [`Publisher::timeout_instance`]. Effect traits (`N`, `M`, `P`, `L`) may be
/// invoked while internal locks are held, so implementations must not block.
pub struct Publisher<P, M, L, N>
where
    P: PublisherProver,
    M: PublisherMessenger,
    L: L1Publisher<SuperblockProof = P::SuperblockProof>,
    N: PublisherNetwork + Clone,
{
    inner: Mutex<PublisherState<P::ChainProof, N>>,
    prover: P,
    messenger: M,
    l1: L,
    network: N,
}

impl<P, M, L, N> std::fmt::Debug for Publisher<P, M, L, N>
where
    P: PublisherProver,
    M: PublisherMessenger,
    L: L1Publisher<SuperblockProof = P::SuperblockProof>,
    N: PublisherNetwork + Clone,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Publisher").finish_non_exhaustive()
    }
}

impl<P, M, L, N> Publisher<P, M, L, N>
where
    P: PublisherProver,
    M: PublisherMessenger,
    L: L1Publisher<SuperblockProof = P::SuperblockProof>,
    N: PublisherNetwork + Clone,
{
    /// Creates a new Publisher. Pass the *previous* period ID and target superblock number.
    /// Call `start_period()` to begin the first period.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prover: P,
        messenger: M,
        l1: L,
        network: N,
        previous_period_id: PeriodId,
        previous_target_superblock_number: SuperblockNumber,
        last_finalized_superblock_number: SuperblockNumber,
        last_finalized_superblock_hash: SuperblockHash,
        proof_window: u64,
        chains: HashSet<ChainId>,
    ) -> Result<Self, PublisherError> {
        if previous_target_superblock_number < last_finalized_superblock_number {
            return Err(PublisherError::InvalidInitialState);
        }

        Ok(Self {
            inner: Mutex::new(PublisherState {
                period_id: previous_period_id,
                target_superblock_number: previous_target_superblock_number,
                last_finalized_superblock_number,
                last_finalized_superblock_hash,
                proofs: HashMap::new(),
                chains,
                sequence_number: SequenceNumber(0),
                active_chains: HashSet::new(),
                instances: HashMap::new(),
                proof_window,
            }),
            prover,
            messenger,
            l1,
            network,
        })
    }

    /// Starts a new period. Resets the sequence number and broadcasts `StartPeriod`.
    pub fn start_period(&self) -> Result<(), PublisherError> {
        let mut state = self.inner.lock().unwrap();

        let next_superblock = state.target_superblock_number + 1;

        // Proof window constraint
        if state.proof_window != 0 {
            let limit =
                state.last_finalized_superblock_number + SuperblockNumber(1 + state.proof_window);
            if next_superblock > limit {
                return Err(PublisherError::CannotStartPeriod {
                    target: state.target_superblock_number.get(),
                    expected: (state.last_finalized_superblock_number + 1).get(),
                });
            }
        }

        state.period_id = state.period_id + 1;
        state.target_superblock_number = next_superblock;

        info!(
            new_period_id = state.period_id.get(),
            target_superblock_number = state.target_superblock_number.get(),
            "Starting new period"
        );

        self.messenger
            .broadcast_start_period(state.period_id, state.target_superblock_number);

        state.sequence_number = SequenceNumber(0);
        Ok(())
    }

    /// Receives a proof from a sequencer. Aggregates and publishes when all
    /// proofs are collected.
    pub fn receive_proof(
        &self,
        period_id: PeriodId,
        superblock_number: SuperblockNumber,
        proof: P::ChainProof,
        chain_id: ChainId,
    ) -> Result<ProofStatus, PublisherError> {
        let mut state = self.inner.lock().unwrap();

        if !state.chains.contains(&chain_id) {
            warn!(
                superblock_number = superblock_number.get(),
                chain_id = chain_id.get(),
                "Received proof from unknown chain, ignoring"
            );
            return Err(PublisherError::ProofFromUnknownChain);
        }

        if superblock_number <= state.last_finalized_superblock_number {
            warn!(
                superblock_number = superblock_number.get(),
                chain_id = chain_id.get(),
                "Received proof for old superblock, ignoring"
            );
            return Err(PublisherError::ProofForFinalizedSuperblock);
        }

        if superblock_number >= state.target_superblock_number {
            warn!(
                superblock_number = superblock_number.get(),
                chain_id = chain_id.get(),
                "Received proof for non-terminated superblock, ignoring"
            );
            return Err(PublisherError::ProofForNonTerminatedSuperblock);
        }

        if superblock_number != state.last_finalized_superblock_number + 1 {
            warn!(
                superblock_number = superblock_number.get(),
                chain_id = chain_id.get(),
                "Received proof for superblock that is not the next one, ignoring"
            );
            return Err(PublisherError::ProofNotSequential);
        }

        // Check period is correct
        let period_diff = state.target_superblock_number - superblock_number;
        let expected_period = state.period_id - PeriodId(period_diff.get());
        if period_id != expected_period {
            warn!(
                superblock_number = superblock_number.get(),
                chain_id = chain_id.get(),
                expected_period = expected_period.get(),
                received_period = period_id.get(),
                "Received proof for wrong period, ignoring"
            );
            return Err(PublisherError::ProofWrongPeriod {
                expected: expected_period.get(),
                received: period_id.get(),
            });
        }

        // Duplicate check
        let sb_proofs = state.proofs.entry(superblock_number).or_default();
        if sb_proofs.contains_key(&chain_id) {
            warn!(
                superblock_number = superblock_number.get(),
                chain_id = chain_id.get(),
                "Already received proof, ignoring"
            );
            return Err(PublisherError::DuplicateProof);
        }

        sb_proofs.insert(chain_id, proof);

        let required = state.chains.len();
        let received = state.proofs[&superblock_number].len();

        if received < required {
            info!(
                superblock_number = superblock_number.get(),
                chain_id = chain_id.get(),
                received,
                required,
                "Received proof, waiting for more"
            );
            return Ok(ProofStatus::Collected { received, required });
        }

        info!(
            superblock_number = superblock_number.get(),
            chain_id = chain_id.get(),
            "Received enough proofs, generating proof"
        );

        let chain_proofs = state.proofs.remove(&superblock_number).unwrap_or_default();
        let last_superblock_hash = state.last_finalized_superblock_hash;
        drop(state);

        match self.prover.request_superblock_proof(
            superblock_number,
            last_superblock_hash,
            chain_proofs,
        ) {
            Ok(superblock_proof) => {
                self.l1.publish_proof(superblock_number, superblock_proof);
                Ok(ProofStatus::Published)
            }
            Err(e) => {
                error!(
                    err = %e,
                    superblock_number = superblock_number.get(),
                    chain_id = chain_id.get(),
                    "Failed to generate network proof. Triggering rollback"
                );
                self.rollback();
                Err(PublisherError::ProverFailed(e.to_string()))
            }
        }
    }

    /// Starts a new SCP instance for the given cross-chain transaction request
    /// and broadcasts `StartInstance` to all participants.
    pub fn start_instance(&self, request: XtRequest) -> Result<Instance, PublisherError> {
        let mut state = self.inner.lock().unwrap();

        let chains = chains_from_request(&request);
        if chains.len() < 2 {
            return Err(PublisherError::InvalidRequest);
        }

        if chains.iter().any(|c| state.active_chains.contains(c)) {
            return Err(PublisherError::CannotStartInstance);
        }

        state.sequence_number = state.sequence_number + 1;
        let instance = Instance {
            id: generate_instance_id(state.period_id, state.sequence_number, &request),
            period_id: state.period_id,
            sequence_number: state.sequence_number,
            xt_request: request,
        };

        for &chain_id in &chains {
            state.active_chains.insert(chain_id);
        }

        let machine = Arc::new(PublisherInstance::new(
            instance.clone(),
            self.network.clone(),
        ));
        state.instances.insert(
            instance.id,
            InstanceEntry {
                machine: Arc::clone(&machine),
                chains,
            },
        );

        info!(
            instance_id = %instance.id,
            period_id = instance.period_id.get(),
            sequence_number = instance.sequence_number.get(),
            "Starting new instance"
        );

        machine.run();
        Ok(instance)
    }

    /// Routes a vote to its SCP instance. On a decision, the instance is
    /// removed and its chains released.
    pub fn process_vote(
        &self,
        instance_id: InstanceId,
        sender: ChainId,
        vote: bool,
    ) -> Result<DecisionState, PublisherError> {
        let machine = self.machine_for(instance_id)?;

        machine.process_vote(sender, vote)?;

        let decision = machine.decision_state();
        if decision != DecisionState::Pending {
            self.finish_instance(instance_id);
        }
        Ok(decision)
    }

    /// Aborts a stale SCP instance, broadcasting `Decided(false)` and
    /// releasing its chains. Scheduling is left to the consumer.
    pub fn timeout_instance(&self, instance_id: InstanceId) -> Result<(), PublisherError> {
        let machine = self.machine_for(instance_id)?;
        machine.timeout();
        self.finish_instance(instance_id);
        Ok(())
    }

    fn machine_for(
        &self,
        instance_id: InstanceId,
    ) -> Result<Arc<PublisherInstance<N>>, PublisherError> {
        let state = self.inner.lock().unwrap();
        state
            .instances
            .get(&instance_id)
            .map(|entry| Arc::clone(&entry.machine))
            .ok_or(PublisherError::UnknownInstance)
    }

    fn finish_instance(&self, instance_id: InstanceId) {
        let mut state = self.inner.lock().unwrap();
        if let Some(entry) = state.instances.remove(&instance_id) {
            for chain_id in &entry.chains {
                state.active_chains.remove(chain_id);
            }
            info!(instance_id = %instance_id, "Instance finished, releasing chains");
        }
    }

    /// Advances the settled state when L1 emits a new settled event.
    pub fn advance_settled_state(
        &self,
        superblock_number: SuperblockNumber,
        superblock_hash: SuperblockHash,
    ) -> Result<(), PublisherError> {
        let mut state = self.inner.lock().unwrap();

        if superblock_number <= state.last_finalized_superblock_number {
            return Err(PublisherError::OldSettledState);
        }

        info!(
            new_finalized_superblock_number = superblock_number.get(),
            "Advancing finalized settled state"
        );

        state.last_finalized_superblock_number = superblock_number;
        state.last_finalized_superblock_hash = superblock_hash;
        Ok(())
    }

    /// Triggers a rollback to the last finalized superblock.
    pub fn proof_timeout(&self) {
        info!("Proof timeout occurred, rolling back to last finalized superblock");
        self.rollback();
    }

    /// Replaces the set of chains whose proofs are required to settle a superblock.
    ///
    /// Membership changes apply to subsequent proof-completeness checks; proofs
    /// already collected for in-flight superblocks are kept and re-evaluated
    /// against the new set on the next [`Self::receive_proof`].
    pub fn update_chains(&self, chains: HashSet<ChainId>) {
        let mut state = self.inner.lock().unwrap();
        info!(chains = chains.len(), "Updating chain set");
        state.chains = chains;
    }

    /// Rolls back the network to the last finalized superblock, e.g. when the
    /// settlement pipeline fails after proof aggregation. Abandons all
    /// in-flight SCP instances.
    pub fn rollback(&self) {
        let mut state = self.inner.lock().unwrap();
        if !state.instances.is_empty() {
            warn!(
                abandoned = state.instances.len(),
                "Abandoning in-flight instances due to rollback"
            );
        }
        state.instances.clear();
        state.active_chains.clear();
        state.sequence_number = SequenceNumber(0);
        state.target_superblock_number = state.last_finalized_superblock_number + 1;
        self.messenger.broadcast_rollback(
            state.period_id,
            state.last_finalized_superblock_number,
            state.last_finalized_superblock_hash,
        );
        state.proofs.clear();
    }

    /// Access the internal target superblock number (for testing).
    #[must_use]
    pub fn target_superblock_number(&self) -> SuperblockNumber {
        self.inner.lock().unwrap().target_superblock_number
    }

    /// Access the current period ID.
    #[must_use]
    pub fn period_id(&self) -> PeriodId {
        self.inner.lock().unwrap().period_id
    }

    /// Access the last finalized superblock number.
    #[must_use]
    pub fn last_finalized_superblock_number(&self) -> SuperblockNumber {
        self.inner.lock().unwrap().last_finalized_superblock_number
    }

    /// The terminated superblock currently awaiting proofs, if any.
    #[must_use]
    pub fn settling_superblock(&self) -> Option<SuperblockNumber> {
        let state = self.inner.lock().unwrap();
        let next = state.last_finalized_superblock_number + 1;
        (next < state.target_superblock_number).then_some(next)
    }

    /// Number of in-flight SCP instances.
    #[must_use]
    pub fn active_instance_count(&self) -> usize {
        self.inner.lock().unwrap().instances.len()
    }

    /// Number of chains currently reserved by in-flight instances.
    #[must_use]
    pub fn active_chain_count(&self) -> usize {
        self.inner.lock().unwrap().active_chains.len()
    }

    /// Chains that have reported a proof for the given superblock.
    #[must_use]
    pub fn proof_chains_for(&self, sb: SuperblockNumber) -> Option<Vec<ChainId>> {
        self.inner
            .lock()
            .unwrap()
            .proofs
            .get(&sb)
            .map(|proofs| proofs.keys().copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct FakeMessenger {
        start_periods: Mutex<Vec<(PeriodId, SuperblockNumber)>>,
        rollbacks: Mutex<Vec<(PeriodId, SuperblockNumber, SuperblockHash)>>,
    }

    impl PublisherMessenger for Arc<FakeMessenger> {
        fn broadcast_start_period(&self, p: PeriodId, t: SuperblockNumber) {
            self.start_periods.lock().unwrap().push((p, t));
        }
        fn broadcast_rollback(&self, p: PeriodId, s: SuperblockNumber, h: SuperblockHash) {
            self.rollbacks.lock().unwrap().push((p, s, h));
        }
    }

    type ProverCall = (SuperblockNumber, SuperblockHash, HashMap<ChainId, Vec<u8>>);

    #[derive(Debug, Default)]
    struct FakeProver {
        calls: Mutex<Vec<ProverCall>>,
        next_proof: Mutex<Vec<u8>>,
        err: Mutex<Option<String>>,
    }

    impl PublisherProver for Arc<FakeProver> {
        type ChainProof = Vec<u8>;
        type SuperblockProof = Vec<u8>;

        fn request_superblock_proof(
            &self,
            sb: SuperblockNumber,
            hash: SuperblockHash,
            proofs: HashMap<ChainId, Vec<u8>>,
        ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
            self.calls.lock().unwrap().push((sb, hash, proofs));
            if let Some(ref e) = *self.err.lock().unwrap() {
                return Err(e.clone().into());
            }
            Ok(self.next_proof.lock().unwrap().clone())
        }
    }

    #[derive(Debug, Default)]
    struct FakeL1 {
        published: Mutex<Vec<(SuperblockNumber, Vec<u8>)>>,
    }

    impl L1Publisher for Arc<FakeL1> {
        type SuperblockProof = Vec<u8>;

        fn publish_proof(&self, sb: SuperblockNumber, proof: Vec<u8>) {
            self.published.lock().unwrap().push((sb, proof));
        }
    }

    #[derive(Debug, Default, Clone)]
    struct FakeNetwork {
        starts: Arc<Mutex<Vec<InstanceId>>>,
        decisions: Arc<Mutex<Vec<(InstanceId, bool)>>>,
    }

    impl PublisherNetwork for FakeNetwork {
        fn send_start_instance(&self, instance: &Instance) {
            self.starts.lock().unwrap().push(instance.id);
        }
        fn send_decided(&self, instance_id: InstanceId, decided: bool) {
            self.decisions.lock().unwrap().push((instance_id, decided));
        }
    }

    fn chain_req(chain: u64, txs: &[&[u8]]) -> ethera_spec::TransactionRequest {
        ethera_spec::TransactionRequest {
            chain_id: ChainId(chain),
            transactions: txs.iter().map(|t| t.to_vec()).collect(),
        }
    }

    fn make_xt_request(entries: Vec<ethera_spec::TransactionRequest>) -> XtRequest {
        XtRequest {
            transactions: entries,
        }
    }

    fn make_chain_set(ids: &[u64]) -> HashSet<ChainId> {
        ids.iter().map(|&id| ChainId(id)).collect()
    }

    fn default_chain_set() -> HashSet<ChainId> {
        make_chain_set(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10])
    }

    type TestPublisher = Publisher<Arc<FakeProver>, Arc<FakeMessenger>, Arc<FakeL1>, FakeNetwork>;

    struct TestHarness {
        publisher: TestPublisher,
        messenger: Arc<FakeMessenger>,
        prover: Arc<FakeProver>,
        l1: Arc<FakeL1>,
        network: FakeNetwork,
    }

    fn new_publisher_for_test(
        period: u64,
        target: u64,
        finalized: u64,
        hash: SuperblockHash,
        window: u64,
        chains: HashSet<ChainId>,
    ) -> TestHarness {
        let messenger = Arc::new(FakeMessenger::default());
        let prover = Arc::new(FakeProver::default());
        let l1 = Arc::new(FakeL1::default());
        let network = FakeNetwork::default();
        let publisher = Publisher::new(
            Arc::clone(&prover),
            Arc::clone(&messenger),
            Arc::clone(&l1),
            network.clone(),
            PeriodId(period),
            SuperblockNumber(target),
            SuperblockNumber(finalized),
            hash,
            window,
            chains,
        )
        .unwrap();
        TestHarness {
            publisher,
            messenger,
            prover,
            l1,
            network,
        }
    }

    #[test]
    fn rejects_target_lower_than_finalized() {
        let result = TestPublisher::new(
            Arc::new(FakeProver::default()),
            Arc::new(FakeMessenger::default()),
            Arc::new(FakeL1::default()),
            FakeNetwork::default(),
            PeriodId(3),
            SuperblockNumber(4),
            SuperblockNumber(5),
            SuperblockHash([1; 32]),
            0,
            default_chain_set(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn start_period_respects_explicit_target() {
        let h = new_publisher_for_test(4, 10, 7, SuperblockHash([5; 32]), 0, default_chain_set());

        h.publisher.start_period().unwrap();
        let starts = h.messenger.start_periods.lock().unwrap();
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0].0, PeriodId(5));
        assert_eq!(starts[0].1, SuperblockNumber(11));
        drop(starts);

        assert_eq!(h.publisher.target_superblock_number(), SuperblockNumber(11));
    }

    #[test]
    fn start_period_error_when_target_exceeds_proof_window() {
        let h = new_publisher_for_test(5, 7, 7, SuperblockHash([1; 32]), 1, default_chain_set());

        h.publisher.start_period().unwrap();
        h.publisher.start_period().unwrap();

        let err = h.publisher.start_period().unwrap_err();
        assert!(matches!(err, PublisherError::CannotStartPeriod { .. }));
        assert_eq!(h.messenger.start_periods.lock().unwrap().len(), 2);
    }

    #[test]
    fn start_period_no_window_constraint_when_disabled() {
        let h = new_publisher_for_test(5, 7, 7, SuperblockHash([1; 32]), 0, default_chain_set());

        for _ in 0..3 {
            h.publisher.start_period().unwrap();
        }

        assert_eq!(h.messenger.start_periods.lock().unwrap().len(), 3);
    }

    #[test]
    fn start_instance_broadcasts_and_reserves() {
        let h = new_publisher_for_test(5, 5, 5, SuperblockHash([1; 32]), 0, default_chain_set());

        let req1 = make_xt_request(vec![chain_req(1, &[b"a"]), chain_req(2, &[b"b"])]);
        let inst1 = h.publisher.start_instance(req1).unwrap();
        assert_eq!(h.network.starts.lock().unwrap().as_slice(), &[inst1.id]);
        assert_eq!(h.publisher.active_instance_count(), 1);
        assert_eq!(h.publisher.active_chain_count(), 2);

        // Disjoint {3,4} should be allowed
        let req2 = make_xt_request(vec![chain_req(3, &[b"c"]), chain_req(4, &[b"d"])]);
        h.publisher.start_instance(req2).unwrap();
        assert_eq!(h.publisher.active_instance_count(), 2);
        assert_eq!(h.publisher.active_chain_count(), 4);

        // Overlapping {2,5} is rejected
        let req3 = make_xt_request(vec![chain_req(2, &[b"x"]), chain_req(5, &[b"y"])]);
        let err = h.publisher.start_instance(req3).unwrap_err();
        assert!(matches!(err, PublisherError::CannotStartInstance));
    }

    #[test]
    fn start_instance_invalid_requests() {
        let h = new_publisher_for_test(1, 1, 1, SuperblockHash([1; 32]), 0, default_chain_set());

        // Empty request
        let err = h
            .publisher
            .start_instance(XtRequest::default())
            .unwrap_err();
        assert!(matches!(err, PublisherError::InvalidRequest));

        // Single-transaction request
        let err = h
            .publisher
            .start_instance(make_xt_request(vec![chain_req(1, &[b"only"])]))
            .unwrap_err();
        assert!(matches!(err, PublisherError::InvalidRequest));

        // Two transaction requests on the same chain are not cross-chain
        let err = h
            .publisher
            .start_instance(make_xt_request(vec![
                chain_req(1, &[b"a"]),
                chain_req(1, &[b"b"]),
            ]))
            .unwrap_err();
        assert!(matches!(err, PublisherError::InvalidRequest));
    }

    #[test]
    fn sequence_monotonic_and_resets_per_period() {
        let h = new_publisher_for_test(10, 9, 9, SuperblockHash([1; 32]), 0, default_chain_set());

        let i1 = h
            .publisher
            .start_instance(make_xt_request(vec![
                chain_req(1, &[b"a1"]),
                chain_req(2, &[b"a2"]),
            ]))
            .unwrap();
        let i2 = h
            .publisher
            .start_instance(make_xt_request(vec![
                chain_req(3, &[b"b1"]),
                chain_req(4, &[b"b2"]),
            ]))
            .unwrap();

        assert_eq!(i1.sequence_number, SequenceNumber(1));
        assert_eq!(i2.sequence_number, SequenceNumber(2));

        h.publisher.start_period().unwrap();

        let i3 = h
            .publisher
            .start_instance(make_xt_request(vec![
                chain_req(5, &[b"c1"]),
                chain_req(6, &[b"c2"]),
            ]))
            .unwrap();
        assert_eq!(i3.sequence_number, SequenceNumber(1));
    }

    #[test]
    fn unanimous_votes_decide_and_release_chains() {
        let h = new_publisher_for_test(1, 1, 1, SuperblockHash([1; 32]), 0, default_chain_set());
        let inst = h
            .publisher
            .start_instance(make_xt_request(vec![
                chain_req(1, &[b"a"]),
                chain_req(2, &[b"b"]),
            ]))
            .unwrap();

        let state = h.publisher.process_vote(inst.id, ChainId(1), true).unwrap();
        assert_eq!(state, DecisionState::Pending);

        let state = h.publisher.process_vote(inst.id, ChainId(2), true).unwrap();
        assert_eq!(state, DecisionState::Accepted);
        assert_eq!(
            h.network.decisions.lock().unwrap().as_slice(),
            &[(inst.id, true)]
        );
        assert_eq!(h.publisher.active_instance_count(), 0);
        assert_eq!(h.publisher.active_chain_count(), 0);

        // Instance is gone: further votes report UnknownInstance.
        let err = h
            .publisher
            .process_vote(inst.id, ChainId(1), true)
            .unwrap_err();
        assert!(matches!(err, PublisherError::UnknownInstance));

        // Chains are free again.
        h.publisher
            .start_instance(make_xt_request(vec![
                chain_req(1, &[b"c"]),
                chain_req(2, &[b"d"]),
            ]))
            .unwrap();
    }

    #[test]
    fn reject_vote_decides_false_immediately() {
        let h = new_publisher_for_test(1, 1, 1, SuperblockHash([1; 32]), 0, default_chain_set());
        let inst = h
            .publisher
            .start_instance(make_xt_request(vec![
                chain_req(1, &[b"a"]),
                chain_req(2, &[b"b"]),
                chain_req(3, &[b"c"]),
            ]))
            .unwrap();

        let state = h
            .publisher
            .process_vote(inst.id, ChainId(2), false)
            .unwrap();
        assert_eq!(state, DecisionState::Rejected);
        assert_eq!(
            h.network.decisions.lock().unwrap().as_slice(),
            &[(inst.id, false)]
        );
        assert_eq!(h.publisher.active_chain_count(), 0);
    }

    #[test]
    fn duplicate_and_non_participant_votes_error() {
        let h = new_publisher_for_test(1, 1, 1, SuperblockHash([1; 32]), 0, default_chain_set());
        let inst = h
            .publisher
            .start_instance(make_xt_request(vec![
                chain_req(1, &[b"a"]),
                chain_req(2, &[b"b"]),
            ]))
            .unwrap();

        h.publisher.process_vote(inst.id, ChainId(1), true).unwrap();
        let err = h
            .publisher
            .process_vote(inst.id, ChainId(1), true)
            .unwrap_err();
        assert!(matches!(err, PublisherError::Scp(_)));

        let err = h
            .publisher
            .process_vote(inst.id, ChainId(99), true)
            .unwrap_err();
        assert!(matches!(err, PublisherError::Scp(_)));

        // Instance still pending and intact.
        assert_eq!(h.publisher.active_instance_count(), 1);
    }

    #[test]
    fn timeout_instance_rejects_and_releases() {
        let h = new_publisher_for_test(1, 1, 1, SuperblockHash([1; 32]), 0, default_chain_set());
        let inst = h
            .publisher
            .start_instance(make_xt_request(vec![
                chain_req(1, &[b"a"]),
                chain_req(2, &[b"b"]),
            ]))
            .unwrap();

        h.publisher.timeout_instance(inst.id).unwrap();
        assert_eq!(
            h.network.decisions.lock().unwrap().as_slice(),
            &[(inst.id, false)]
        );
        assert_eq!(h.publisher.active_instance_count(), 0);
        assert_eq!(h.publisher.active_chain_count(), 0);

        let err = h.publisher.timeout_instance(inst.id).unwrap_err();
        assert!(matches!(err, PublisherError::UnknownInstance));
    }

    #[test]
    fn rollback_abandons_instances_without_decisions() {
        let h = new_publisher_for_test(3, 9, 6, SuperblockHash([4; 32]), 0, default_chain_set());
        h.publisher
            .start_instance(make_xt_request(vec![
                chain_req(1, &[b"a"]),
                chain_req(2, &[b"b"]),
            ]))
            .unwrap();

        h.publisher.rollback();

        assert_eq!(h.publisher.active_instance_count(), 0);
        assert_eq!(h.publisher.active_chain_count(), 0);
        assert!(h.network.decisions.lock().unwrap().is_empty());
        let rollbacks = h.messenger.rollbacks.lock().unwrap();
        assert_eq!(rollbacks.len(), 1);
        assert_eq!(rollbacks[0].1, SuperblockNumber(6));
        drop(rollbacks);
        assert_eq!(h.publisher.target_superblock_number(), SuperblockNumber(7));
    }

    #[test]
    fn advance_settled_state_monotonic() {
        let h = new_publisher_for_test(1, 1, 1, SuperblockHash([1; 32]), 0, default_chain_set());

        h.publisher
            .advance_settled_state(SuperblockNumber(2), SuperblockHash([9; 32]))
            .unwrap();

        let err = h
            .publisher
            .advance_settled_state(SuperblockNumber(2), SuperblockHash([8; 32]))
            .unwrap_err();
        assert!(matches!(err, PublisherError::OldSettledState));
    }

    #[test]
    fn proof_timeout_rolls_back_and_resets_target() {
        let finalized = SuperblockNumber(5);
        let h = new_publisher_for_test(
            3,
            finalized.get(),
            finalized.get(),
            SuperblockHash([7; 32]),
            0,
            default_chain_set(),
        );

        h.publisher.proof_timeout();

        let rollbacks = h.messenger.rollbacks.lock().unwrap();
        assert_eq!(rollbacks.len(), 1);
        assert_eq!(rollbacks[0].0, PeriodId(3));
        assert_eq!(rollbacks[0].1, finalized);
        assert_eq!(rollbacks[0].2, SuperblockHash([7; 32]));
        drop(rollbacks);

        assert_eq!(h.publisher.target_superblock_number(), finalized + 1);
    }

    #[test]
    fn receive_proof_aggregates_and_publishes() {
        let h = new_publisher_for_test(
            10,
            5,
            5,
            SuperblockHash([1; 32]),
            0,
            make_chain_set(&[1, 2]),
        );
        *h.prover.next_proof.lock().unwrap() = b"network-proof".to_vec();
        h.publisher.start_period().unwrap();
        h.publisher.start_period().unwrap();

        let status = h
            .publisher
            .receive_proof(
                PeriodId(11),
                SuperblockNumber(6),
                b"proof-1".to_vec(),
                ChainId(1),
            )
            .unwrap();
        assert_eq!(
            status,
            ProofStatus::Collected {
                received: 1,
                required: 2
            }
        );
        assert!(h.l1.published.lock().unwrap().is_empty());

        let status = h
            .publisher
            .receive_proof(
                PeriodId(11),
                SuperblockNumber(6),
                b"proof-2".to_vec(),
                ChainId(2),
            )
            .unwrap();
        assert_eq!(status, ProofStatus::Published);

        let calls = h.prover.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, SuperblockNumber(6));
        assert_eq!(calls[0].1, SuperblockHash([1; 32]));
        assert_eq!(calls[0].2.len(), 2);
        assert_eq!(calls[0].2[&ChainId(1)], b"proof-1".to_vec());
        drop(calls);

        let published = h.l1.published.lock().unwrap();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].0, SuperblockNumber(6));
        assert_eq!(published[0].1, b"network-proof".to_vec());
        drop(published);

        assert!(h.publisher.proof_chains_for(SuperblockNumber(6)).is_none());
    }

    #[test]
    fn receive_proof_rejections() {
        let h = new_publisher_for_test(
            10,
            5,
            5,
            SuperblockHash([1; 32]),
            0,
            make_chain_set(&[1, 2]),
        );
        h.publisher.start_period().unwrap();
        h.publisher.start_period().unwrap();

        // Unknown chain
        let err = h
            .publisher
            .receive_proof(PeriodId(11), SuperblockNumber(6), b"p".to_vec(), ChainId(9))
            .unwrap_err();
        assert!(matches!(err, PublisherError::ProofFromUnknownChain));

        // Old superblock
        let err = h
            .publisher
            .receive_proof(PeriodId(11), SuperblockNumber(5), b"p".to_vec(), ChainId(1))
            .unwrap_err();
        assert!(matches!(err, PublisherError::ProofForFinalizedSuperblock));

        // Non-terminated superblock
        let err = h
            .publisher
            .receive_proof(PeriodId(12), SuperblockNumber(7), b"p".to_vec(), ChainId(1))
            .unwrap_err();
        assert!(matches!(
            err,
            PublisherError::ProofForNonTerminatedSuperblock
        ));

        // Wrong period
        let err = h
            .publisher
            .receive_proof(PeriodId(12), SuperblockNumber(6), b"p".to_vec(), ChainId(1))
            .unwrap_err();
        assert!(matches!(err, PublisherError::ProofWrongPeriod { .. }));

        // Duplicate
        h.publisher
            .receive_proof(PeriodId(11), SuperblockNumber(6), b"p".to_vec(), ChainId(1))
            .unwrap();
        let err = h
            .publisher
            .receive_proof(PeriodId(11), SuperblockNumber(6), b"p".to_vec(), ChainId(1))
            .unwrap_err();
        assert!(matches!(err, PublisherError::DuplicateProof));
    }

    #[test]
    fn receive_proof_prover_error_triggers_rollback() {
        let h = new_publisher_for_test(
            10,
            5,
            5,
            SuperblockHash([9; 32]),
            0,
            make_chain_set(&[1, 2]),
        );
        *h.prover.err.lock().unwrap() = Some("boom".into());
        h.publisher.start_period().unwrap();
        h.publisher.start_period().unwrap();

        h.publisher
            .receive_proof(
                PeriodId(11),
                SuperblockNumber(6),
                b"proof-1".to_vec(),
                ChainId(1),
            )
            .unwrap();
        let err = h
            .publisher
            .receive_proof(
                PeriodId(11),
                SuperblockNumber(6),
                b"proof-2".to_vec(),
                ChainId(2),
            )
            .unwrap_err();
        assert!(matches!(err, PublisherError::ProverFailed(_)));

        assert!(h.l1.published.lock().unwrap().is_empty());
        let rollbacks = h.messenger.rollbacks.lock().unwrap();
        assert_eq!(rollbacks.len(), 1);
        assert_eq!(rollbacks[0].1, SuperblockNumber(5));
        assert_eq!(rollbacks[0].2, SuperblockHash([9; 32]));
    }

    #[test]
    fn update_chains_changes_completeness_threshold() {
        let h = new_publisher_for_test(
            10,
            5,
            5,
            SuperblockHash([1; 32]),
            0,
            make_chain_set(&[1, 2, 3]),
        );
        *h.prover.next_proof.lock().unwrap() = b"network-proof".to_vec();
        h.publisher.start_period().unwrap();
        h.publisher.start_period().unwrap();

        h.publisher
            .receive_proof(
                PeriodId(11),
                SuperblockNumber(6),
                b"proof-1".to_vec(),
                ChainId(1),
            )
            .unwrap();
        assert!(h.l1.published.lock().unwrap().is_empty());

        // Shrinking the chain set to {1, 2} makes the second proof complete the set.
        h.publisher.update_chains(make_chain_set(&[1, 2]));
        let status = h
            .publisher
            .receive_proof(
                PeriodId(11),
                SuperblockNumber(6),
                b"proof-2".to_vec(),
                ChainId(2),
            )
            .unwrap();
        assert_eq!(status, ProofStatus::Published);

        assert_eq!(h.l1.published.lock().unwrap().len(), 1);
    }

    #[test]
    fn settling_superblock_tracks_terminated_superblocks() {
        let h = new_publisher_for_test(4, 7, 7, SuperblockHash([2; 32]), 0, default_chain_set());
        assert_eq!(h.publisher.settling_superblock(), None);

        h.publisher.start_period().unwrap();
        // Superblock 8 is being built, nothing terminated yet.
        assert_eq!(h.publisher.settling_superblock(), None);

        h.publisher.start_period().unwrap();
        // Superblock 8 terminated, awaiting proofs.
        assert_eq!(h.publisher.settling_superblock(), Some(SuperblockNumber(8)));

        h.publisher
            .advance_settled_state(SuperblockNumber(8), SuperblockHash([3; 32]))
            .unwrap();
        assert_eq!(h.publisher.settling_superblock(), None);
    }

    #[test]
    fn state_accessors_track_period_and_finalized() {
        let h = new_publisher_for_test(4, 7, 7, SuperblockHash([2; 32]), 0, default_chain_set());

        assert_eq!(h.publisher.period_id(), PeriodId(4));
        assert_eq!(
            h.publisher.last_finalized_superblock_number(),
            SuperblockNumber(7)
        );

        h.publisher.start_period().unwrap();
        assert_eq!(h.publisher.period_id(), PeriodId(5));

        h.publisher
            .advance_settled_state(SuperblockNumber(8), SuperblockHash([3; 32]))
            .unwrap();
        assert_eq!(
            h.publisher.last_finalized_superblock_number(),
            SuperblockNumber(8)
        );
    }
}
