use std::fmt;

use crate::{ChainId, InstanceId, PeriodId, SequenceNumber};

/// Decision state for an SCP instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DecisionState {
    #[default]
    Pending,
    Accepted,
    Rejected,
}

impl fmt::Display for DecisionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => f.write_str("Pending"),
            Self::Accepted => f.write_str("Accepted"),
            Self::Rejected => f.write_str("Rejected"),
        }
    }
}

/// A request of multiple transactions for a specific chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRequest {
    pub chain_id: ChainId,
    pub transactions: Vec<Vec<u8>>,
}

/// Cross-chain transaction request with multiple per-chain requests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct XtRequest {
    pub transactions: Vec<TransactionRequest>,
}

/// An SCP instance with its associated cross-chain transaction request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub id: InstanceId,
    pub period_id: PeriodId,
    pub sequence_number: SequenceNumber,
    pub xt_request: XtRequest,
}

impl Instance {
    /// Returns the unique set of chain IDs involved in this instance.
    #[must_use]
    pub fn chains(&self) -> Vec<ChainId> {
        chains_from_request(&self.xt_request)
    }
}

/// Returns the unique set of chain IDs referenced by an `XtRequest`.
#[must_use]
pub fn chains_from_request(xt_request: &XtRequest) -> Vec<ChainId> {
    let mut seen = std::collections::HashSet::new();
    let mut chains = Vec::new();
    for r in &xt_request.transactions {
        if seen.insert(r.chain_id) {
            chains.push(r.chain_id);
        }
    }
    chains
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_from_request_deduplicates() {
        let req = XtRequest {
            transactions: vec![
                TransactionRequest {
                    chain_id: ChainId(1),
                    transactions: vec![vec![1]],
                },
                TransactionRequest {
                    chain_id: ChainId(1),
                    transactions: vec![vec![2]],
                },
                TransactionRequest {
                    chain_id: ChainId(2),
                    transactions: vec![vec![3]],
                },
            ],
        };
        let chains = chains_from_request(&req);
        assert_eq!(chains.len(), 2);
        assert!(chains.contains(&ChainId(1)));
        assert!(chains.contains(&ChainId(2)));
    }

    #[test]
    fn decision_state_display() {
        assert_eq!(DecisionState::Pending.to_string(), "Pending");
        assert_eq!(DecisionState::Accepted.to_string(), "Accepted");
        assert_eq!(DecisionState::Rejected.to_string(), "Rejected");
    }
}
