Feature: Sequencer Decision
  On the sequencer perspective, an instance terminates in two cases:
  - when it sends a rejection vote (either due simulation failure or timeout)
  - when it receives a decision from the SP
  If the instance is decided as rejected, the sequencer should not include its transactions in a block.
  Else, it should include the transactions along with the created mailbox.putInbox ones.
  Note that the shared publisher can only send a decision true if all sequencers have voted true.
  Thus, receiving a decision true without having voted true is an invalid protocol state.

  Background:
    Given there is a chain "1" with sequencer "A"
    And there is a chain "2" with sequencer "B"

  @sequencer @scp @decision @happy-path
  Scenario: Votes false upon simulation failure
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And the execution engine simulates "tx1" and returns an error other than "Read miss"
    When sequencer "A" publishes Vote with:
      | field       | value |
      | instance_id | 0x1   |
      | chain_id    | 1     |
      | vote        | false |
    Then sequencer "A" should mark the instance "0x1" as rejected

  @sequencer @scp @decision @mailbox @happy-path
  Scenario: Forwards mailbox messages produced by a failing simulation before voting false
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    When the execution engine simulates "tx1" and writes a mailbox message before failing with an error other than "Read miss":
      | field             | value       |
      | source_chain      | 1           |
      | destination_chain | 2           |
      | source            | 0xaaa       |
      | receiver          | 0xbbb       |
      | session_id        | 0x777       |
      | label             | TRANSFER    |
      | data              | [0x01,0x02] |
    Then sequencer "A" should forward that MailboxMessage to sequencer "B" with instance ID "0x1"
    And sequencer "A" should publish Vote with:
      | field       | value |
      | instance_id | 0x1   |
      | chain_id    | 1     |
      | vote        | false |

  @sequencer @scp @decision @happy-path
  Scenario: Rejects instance when decision is false
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    When sequencer "A" receives Decided for instance "0x1" with decision "false"
    Then sequencer "A" should mark the instance "0x1" as rejected

  @sequencer @scp @decision @error
  Scenario: Errors when decision true arrives without a prior vote
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And sequencer "A" has not published a Vote for instance "0x1"
    When sequencer "A" receives Decided for instance "0x1" with decision "true"
    Then an error occurs:
      """
      decision true but no vote sent is an impossible state
      """

  @sequencer @scp @decision @error
  Scenario: Errors when decision true contradicts a prior false vote
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And sequencer "A" previously published Vote with:
      | field       | value |
      | instance_id | 0x1   |
      | chain_id    | 1     |
      | vote        | false |
    When sequencer "A" receives Decided for instance "0x1" with decision "true"
    Then an error occurs:
      """
      decision true but previous vote was false is an impossible state
      """

  @sequencer @scp @decision @happy-path
  Scenario: Accepts instance when decision is true after voting true
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And the execution engine simulates "tx1" and returns success
    And sequencer "A" previously published Vote with:
      | field       | value |
      | instance_id | 0x1   |
      | chain_id    | 1     |
      | vote        | true  |
    When sequencer "A" receives Decided for instance "0x1" with decision "true"
    Then sequencer "A" should mark the instance "0x1" as accepted

  @sequencer @scp @decision @happy-path
  Scenario: Rejects instance when decision is false even after voting true
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And the execution engine simulates "tx1" and returns success
    And sequencer "A" previously published Vote with:
      | field       | value |
      | instance_id | 0x1   |
      | chain_id    | 1     |
      | vote        | true  |
    When sequencer "A" receives Decided for instance "0x1" with decision "false"
    Then sequencer "A" should mark the instance "0x1" as rejected

  @sequencer @scp @decision @happy-path
  Scenario: Does not include putInbox transactions in the block when instance is rejected
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x2
      period_id: 3
      sequence_number: 3
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And during simulation a mailbox message was received and a mailbox.putInbox transaction "PItx" was created for instance "0x2"
    When sequencer "A" receives Decided for instance "0x2" with decision "false"
    Then sequencer "A" should mark the instance "0x2" as rejected
    And sequencer "A" should not include "tx1" or the mailbox.putInbox transaction "PItx" in the block

  @sequencer @scp @decision @error
  Scenario: Raises error when a decided instance receives a second decision
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And sequencer "A" receives Decided for instance "0x1" with decision "false"
    When sequencer "A" later receives Decided for instance "0x1" with decision "true"
    Then an error occurs:
      """
      instance already decided
      """
