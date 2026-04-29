Feature: Sequencer Block Policy
  A sequencer must build L2 blocks sequentially, tagging every block with the active period ID
  and target superblock number announced by the publisher.
  While a composability instance is active, local transactions cannot be executed.
  Blocks cannot be closed while an instance is active and
  the sequencer must guarantee only one block is pending at a time.

  Background:
    Given there is a chain "1" with sequencer "A"
    And the sequencer "A" is at period ID "10" targeting superblock "9"

  @sequencer @sbcp @blocks @error
  Scenario Outline: Starting a new block with a non-sequential number is rejected
    Given the sequencer "A" has no pending block
    And the sequencer "A" last closed block number is <last_closed>
    When the sequencer "A" attempts to begin building block <new_block>
    Then the attempt should fail with error:
      """
      block number is not sequential
      """

    Examples:
      | last_closed | new_block |
      | 100         | 100       |
      | 100         | 102       |
      | 100         | 103       |
      | 100         | 99        |

  @sequencer @sbcp @blocks @error
  Scenario Outline: Starting a new block with an already pending block is rejected
    Given the sequencer "A" has a pending block "101"
    When the sequencer "A" attempts to begin building block <new_block>
    Then the attempt should fail with error:
      """
      there is already a pending block
      """
    Examples:
      | new_block |
      | 101       |
      | 102       |
      | 103       |

  @sequencer @sbcp @blocks @happy-path
  Scenario Outline: Successful block beginning
    Given the sequencer "A" has no pending block
    And the sequencer "A" last closed block number is <last_closed>
    When the sequencer "A" begins building block <new_block>
    Then the sequencer "A" should have a new pending block <new_block> with period "10" and superblock "9"
    Examples:
      | last_closed | new_block |
      | 100         | 101       |
      | 101         | 102       |
      | 102         | 103       |

  @sequencer @sbcp @blocks @error
  Scenario: Local transactions are rejected while an instance is active
    Given the sequencer "A" has a pending block "101"
    And the sequencer "A" has an active instance "0xabc"
    When the sequencer "A" attempts to add local transaction "0x1" to block "101"
    Then the attempt should fail with error:
      """
      local transactions are disabled while an instance is active
      """

  @sequencer @sbcp @blocks @happy-path
  Scenario: Local transactions can be added if there is no active instance
    Given the sequencer "A" has a pending block "101"
    And the sequencer "A" has no active instance
    When the sequencer "A" attempts to add local transaction "0x1" to block "101"
    Then the local transaction "0x1" should be added to block "101"

  @sequencer @sbcp @blocks @error
  Scenario: Local transactions are rejected when no block is pending
    Given the sequencer "A" has no pending block
    When the sequencer "A" attempts to add local transaction "0x1" to block "101"
    Then the attempt should fail with error:
      """
      no pending block
      """

  @sequencer @sbcp @blocks @error
  Scenario: Blocks cannot be closed while an instance is active
    Given the sequencer "A" has a pending block "101"
    And the sequencer "A" has an active instance "0xdef"
    When the sequencer "A" attempts to close block "101"
    Then the attempt should fail with error:
      """
      there is already an active instance
      """

  @sequencer @sbcp @blocks @error
  Scenario: Closing without a pending block is rejected
    Given the sequencer "A" has no pending block
    When the sequencer "A" attempts to close block "101"
    Then the attempt should fail with error:
      """
      no pending block
      """

  @sequencer @sbcp @blocks @error
  Scenario: Closing the wrong block number is rejected
    Given the sequencer "A" has a pending block "101"
    And the sequencer "A" has no active instance
    When the sequencer "A" attempts to close block "100"
    Then the attempt should fail with error:
      """
      block number to be closed does not match the current block number
      """

  @sequencer @sbcp @blocks @happy-path
  Scenario: Blocks can be closed if there is no active instance
    Given the sequencer "A" has a pending block "101"
    And the sequencer "A" has no active instance
    When the sequencer "A" attempts to close block "101"
    Then the block "101" should be successfully closed

  @sequencer @sbcp @blocks @happy-path
  Scenario: A pending block's tag is immutable across StartPeriod arrivals
    Given the sequencer "A" has a pending block "101" tagged with period "10" and superblock "9"
    When the sequencer "A" receives StartPeriod:
      | field             | value |
      | period_id         | 11    |
      | target_superblock | 10    |
    Then the sequencer "A" should update its current period to "11" and its target superblock to "10"
    And the pending block "101" should remain tagged with period "10" and superblock "9"
