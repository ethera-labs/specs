use ethera_spec::SequenceNumber;
use thiserror::Error;

/// A `StartInstance` carried a sequence number that did not advance past the
/// last one accepted in the period.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("instance sequence {received} does not advance past {last}")]
pub struct SequenceNotAdvanced {
    pub last: u64,
    pub received: u64,
}

/// Monotonic instance-sequence watermark within a period.
///
/// The publisher assigns each instance a sequence number that strictly
/// increases within a period and resets each period. Like an account nonce, a
/// sequencer keeps the highest sequence it has accepted and rejects any that
/// does not advance, which protects against replayed or reordered
/// `StartInstance` messages. Period boundaries reset the watermark by replacing
/// it with a fresh [`InstanceSequence::default`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InstanceSequence {
    last: Option<SequenceNumber>,
}

impl InstanceSequence {
    /// Accepts `sequence_number` and records it as the new high-water mark, or
    /// rejects it if it does not strictly advance.
    pub fn advance(&mut self, sequence_number: SequenceNumber) -> Result<(), SequenceNotAdvanced> {
        if let Some(last) = self.last {
            if sequence_number <= last {
                return Err(SequenceNotAdvanced {
                    last: last.get(),
                    received: sequence_number.get(),
                });
            }
        }
        self.last = Some(sequence_number);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_strictly_increasing() {
        let mut seq = InstanceSequence::default();
        seq.advance(SequenceNumber(1)).unwrap();
        seq.advance(SequenceNumber(2)).unwrap();
        seq.advance(SequenceNumber(5)).unwrap();
    }

    #[test]
    fn rejects_non_advancing() {
        let mut seq = InstanceSequence::default();
        seq.advance(SequenceNumber(3)).unwrap();
        assert_eq!(
            seq.advance(SequenceNumber(3)),
            Err(SequenceNotAdvanced {
                last: 3,
                received: 3
            })
        );
        assert_eq!(
            seq.advance(SequenceNumber(2)),
            Err(SequenceNotAdvanced {
                last: 3,
                received: 2
            })
        );
    }

    #[test]
    fn default_accepts_any_first_sequence() {
        let mut seq = InstanceSequence::default();
        seq.advance(SequenceNumber(7)).unwrap();
    }
}
