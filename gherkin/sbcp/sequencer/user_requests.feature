Feature: Sequencer User Request Forwarding
  Users can submit XTRequests directly to a sequencer.
  The sequencer's role is purely to relay the request to the SP and discard it locally.
  The sequencer never starts instances on its own — only the SP does.

  Background:
    Given there is a chain "1" with sequencer "A"
    And the sequencer "A" is at period ID "20" targeting superblock "11"

  @sequencer @sbcp @user-requests
  Scenario: Forwards user XTRequests to the SP
    When a user submits an XTRequest to sequencer "A":
      """
      1: [tx1]
      2: [tx2]
      """
    Then the sequencer "A" should forward to the SP the XTRequest:
      """
      1: [tx1]
      2: [tx2]
      """
