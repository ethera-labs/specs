Feature: Sequencer Simulation And Mailbox Population
  After instance start, the sequencer should immediately start the simulation process.
  The simulation should return a success or an error, as well as the written mailbox messages.
  Any new written mailbox message should be forwarded to the destination sequencer.
  In case the result is a success, the sequencer should vote true.
  In case the result is an error other than "Read miss", the sequencer should vote false.
  In case of a "Read miss" error, the sequencer should record such message header as expected and wait for it.
  Once a mailbox message from another sequencer is received via the network, the sequencer should add it to the list
  of pending messages.
  Once the header of a new pending message matches the header of an expected one,
  the sequencer should create a mailbox.putInbox transaction for it and insert it before the other transactions,
  and then restart the simulation.

  Background:
    Given there is a chain "1" with sequencer "A"
    And there is a chain "2" with sequencer "B"

  @sequencer @scp @simulation
  Scenario Outline: Votes according to simulation outcome
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    When the execution engine simulates "tx1" and <result>
    Then sequencer "A" should publish Vote with:
      | field       | value  |
      | instance_id | 0x1    |
      | chain_id    | 1      |
      | vote        | <vote> |
    And no additional MailboxMessage should be forwarded

    Examples:
      | result                                    | vote  |
      | succeeds                                  | true  |
      | an error is raised other than "Read miss" | false |

  @sequencer @scp @simulation @mailbox
  Scenario: Records expected mailbox message after read miss
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    When the execution engine simulates "tx1" and returns a read miss for the mailbox message header:
      | field             | value |
      | source_chain      | 2     |
      | destination_chain | 1     |
      | source            | 0xabc |
      | receiver          | 0xdef |
      | session_id        | 0x123 |
      | label             | MSG   |
    Then sequencer "A" should record that mailbox message as expected for instance "0x1"

  @sequencer @scp @simulation @mailbox
  Scenario: Sends written mailbox message to the destination sequencer
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 3
      sequence_number: 9
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    When the execution engine simulates "tx1" and writes a mailbox message with:
      | field             | value      |
      | source_chain      | 1          |
      | destination_chain | 2          |
      | source            | 0xaaa      |
      | receiver          | 0xbbb      |
      | session_id        | 0x777      |
      | label             | TRANSFER   |
      | data              | [0x01,0x02] |
    Then sequencer "A" should forward that MailboxMessage to sequencer "B" with instance ID "0x1"

  @sequencer @scp @simulation @mailbox
  Scenario: Sends multiple written mailbox messages to the destination sequencer
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 3
      sequence_number: 9
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    When the execution engine simulates "tx1" and writes the following mailbox messages:
      | source_chain | destination_chain | source | receiver | session_id | label     | data        |
      | 1            | 2                 | 0xaaa  | 0xbbb    | 0x777      | TRANSFER  | [0x01,0x02] |
      | 1            | 2                 | 0xccc  | 0xddd    | 0x888      | NOTE      | [0x03]      |
    Then sequencer "A" should forward the mailbox messages to sequencer "B" with instance ID "0x1"

  @sequencer @scp @simulation @mailbox
  Scenario: Does not resend a mailbox message that was already forwarded
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 3
      sequence_number: 9
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And the execution engine simulates "tx1" on the first attempt and writes a mailbox message with:
      | field             | value       |
      | source_chain      | 1           |
      | destination_chain | 2           |
      | source            | 0xaaa       |
      | receiver          | 0xbbb       |
      | session_id        | 0x777       |
      | label             | TRANSFER    |
      | data              | [0x01,0x02] |
    And sequencer "A" has forwarded that MailboxMessage to sequencer "B"
    When the execution engine simulates "tx1" again and writes the same mailbox message
    Then no additional MailboxMessage should be forwarded

  @sequencer @scp @simulation @mailbox
  Scenario: Queues inbound mailbox message when no expected header is recorded
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And sequencer "A" has not recorded any expected mailbox message header for instance "0x1"
    When sequencer "A" receives MailboxMessage with:
      | field             | value |
      | source_chain      | 2     |
      | destination_chain | 1     |
      | source            | 0xabc |
      | receiver          | 0xdef |
      | session_id        | 0x123 |
      | label             | MSG   |
      | instance_id       | 0x1   |
    Then the message should be appended to the pending mailbox queue for instance "0x1"
    And sequencer "A" should not start a new simulation

  @sequencer @scp @simulation @mailbox
  Scenario: Resolves inbound mailbox message matching a recorded expected header
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And sequencer "A" has recorded an expected mailbox message header with:
      | field             | value |
      | source_chain      | 2     |
      | destination_chain | 1     |
      | source            | 0xabc |
      | receiver          | 0xdef |
      | session_id        | 0x123 |
      | label             | MSG   |
    When sequencer "A" receives MailboxMessage with the same header and instance ID "0x1"
    Then the message is removed from the expected set
    And a mailbox.putInbox transaction is added for the message
    And sequencer "A" should start a new simulation

  @sequencer @scp @simulation @mailbox
  Scenario: Records second expected header after read miss on simulation retry
    Given sequencer "A" receives StartInstance:
      """
      instance_id: 0x1
      period_id: 2
      sequence_number: 2
      xtrequest:
        1: [tx1]
        2: [tx2]
      """
    And the execution engine simulates "tx1" on the first attempt and returns a read miss for mailbox message header:
      | field             | value |
      | source_chain      | 2     |
      | destination_chain | 1     |
      | source            | 0xabc |
      | receiver          | 0xdef |
      | session_id        | 0x123 |
      | label             | MSG1  |
    And sequencer "A" receives the MailboxMessage matching header "MSG1" and restarts simulation
    When the execution engine simulates "tx1" on the retry and returns a read miss for a different mailbox message header:
      | field             | value |
      | source_chain      | 2     |
      | destination_chain | 1     |
      | source            | 0xabc |
      | receiver          | 0xdef |
      | session_id        | 0x456 |
      | label             | MSG2  |
    Then sequencer "A" should record the "MSG2" header as expected for instance "0x1"
    And "MSG1" should no longer be in the expected set for instance "0x1"
