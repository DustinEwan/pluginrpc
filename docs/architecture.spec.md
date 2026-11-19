<doc-section name="parse_response">

## What does the mechanism guarantee, and what does a profile still have to decide?

The envelope carries exactly one of `result` or `error`, so `parse_response` refuses a response that carries both <doc-claim name="refuses-a-response-with-both-fields"/>: reading the result would report success while discarding the failure, and refusing the whole message is the only answer that reports neither falsely. What the mechanism leaves to a profile is the method set and the payload behind `result`.
