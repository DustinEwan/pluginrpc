Feature: reading one message

  @id:framing/refuses-a-malformed-line
  Scenario: A line that is not JSON is refused
    Given the fixtures/malformed-line fixture
    When parse-a-request runs on it

  @id:framing/accepts-a-valid-request
  Scenario: A request line is accepted
    Given the fixtures/valid-request fixture
    When parse-a-request runs on it

  @id:framing/blank-line-is-loss
  Scenario: A blank line ends the pipe
    Given the fixtures/blank-line fixture
    When read-a-pipe runs on it
