use ethera_spec::{PeriodId, SequenceNumber};
use thiserror::Error;

/// Ordering error for an inbound `StartInstance` message.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum StartInstanceOrderError {
    /// The message belongs to a period that already ended locally.
    #[error("start-instance period {received} is older than current period {current}")]
    StalePeriod {
        current: PeriodId,
        received: PeriodId,
    },
    /// The message belongs to a period that has not started locally.
    #[error("start-instance period {received} is newer than current period {current}")]
    FuturePeriod {
        current: PeriodId,
        received: PeriodId,
    },
    /// The sequence number does not strictly advance within the period.
    #[error("start-instance sequence {received} does not advance past {last}")]
    SequenceNotAdvanced {
        last: SequenceNumber,
        received: SequenceNumber,
    },
}

/// Validate the SBCP ordering rule for a `StartInstance` message.
///
/// The shared publisher assigns a sequence number that is globally
/// monotonically increasing within a period and resets when the next period
/// starts. Consumers keep their own last accepted sequence because a given
/// rollup may legitimately observe gaps for instances it does not participate
/// in.
pub fn validate_start_instance_order(
    current_period: PeriodId,
    last_sequence_number: Option<SequenceNumber>,
    received_period: PeriodId,
    received_sequence_number: SequenceNumber,
) -> Result<(), StartInstanceOrderError> {
    if received_period < current_period {
        return Err(StartInstanceOrderError::StalePeriod {
            current: current_period,
            received: received_period,
        });
    }

    if received_period > current_period {
        return Err(StartInstanceOrderError::FuturePeriod {
            current: current_period,
            received: received_period,
        });
    }

    if let Some(last) = last_sequence_number {
        if received_sequence_number <= last {
            return Err(StartInstanceOrderError::SequenceNotAdvanced {
                last,
                received: received_sequence_number,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_first_sequence_with_gaps() {
        validate_start_instance_order(PeriodId(7), None, PeriodId(7), SequenceNumber(11)).unwrap();

        validate_start_instance_order(
            PeriodId(7),
            Some(SequenceNumber(11)),
            PeriodId(7),
            SequenceNumber(19),
        )
        .unwrap();
    }

    #[test]
    fn rejects_period_mismatch() {
        assert_eq!(
            validate_start_instance_order(PeriodId(7), None, PeriodId(6), SequenceNumber(1),),
            Err(StartInstanceOrderError::StalePeriod {
                current: PeriodId(7),
                received: PeriodId(6),
            })
        );

        assert_eq!(
            validate_start_instance_order(PeriodId(7), None, PeriodId(8), SequenceNumber(1),),
            Err(StartInstanceOrderError::FuturePeriod {
                current: PeriodId(7),
                received: PeriodId(8),
            })
        );
    }

    #[test]
    fn rejects_non_advancing_sequence() {
        assert_eq!(
            validate_start_instance_order(
                PeriodId(7),
                Some(SequenceNumber(3)),
                PeriodId(7),
                SequenceNumber(3),
            ),
            Err(StartInstanceOrderError::SequenceNotAdvanced {
                last: SequenceNumber(3),
                received: SequenceNumber(3),
            })
        );

        assert_eq!(
            validate_start_instance_order(
                PeriodId(7),
                Some(SequenceNumber(3)),
                PeriodId(7),
                SequenceNumber(2),
            ),
            Err(StartInstanceOrderError::SequenceNotAdvanced {
                last: SequenceNumber(3),
                received: SequenceNumber(2),
            })
        );
    }
}
