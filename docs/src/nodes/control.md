# Control

Nodes for controlling the flow of execution

## Fallback

- The primary node for error handling
- Takes a failure and any other input
- Second input is dynamically typed
- It will not be run unless upstream node produces a failure
- When executed, simply mirrors input downstream
- Used in concert with Select nodes

## Match

- Conditionally send data to one of the output pins
- Use this sparingly and locally to avoid confusion
- Handles text or numeric patterns based on input kind
- Unwraps JSON texts and numbers (but not complex values)
- String match uses [regular expressions](https://docs.rs/regex/latest/regex/) when `exact` disabled
  - Unanchored: `foo` matches "foo", "food" and "buffoon"
  - Anchored: `^foo$` only matches "foo"
- Numeric matching uses ranges when `exact` is disabled
  - Half open intervals match start but not end: `0..100` or `0.5..1.0`
  - Closed intervals match both start and end: `0..=100.0`
- Tests each case from top to bottom
- If no matches, output goes to the `(default)` pin
- Cases can be moved up or down the stack, except for default
- Only the last non-default case can be removed

## Select

- Joins primary and secondary control paths together
- Noteworthy since it is only node that will run with inputs pending
- It emits the first input ready downstream
- Can take any number of inputs of the same kind
- Used with Fallback to produce an alternative path for a fallible subtask
- example:
  - A tool call has a high chance of failure
  - Fallback to an unstructured completion
  - Select them into the Context for a downstream agent

## Demote

- Adjust the priority of a path so it runs later than it would
- In the current implementation, no concurrent node execution
- Long running nodes can delay simple things like Preview and Output
- Use this to ensure that slower tasks yield

## Panic

- Will halt the workflow if it receives a non-empty input
- Can force workflow to still halt after capturing failure with Output
- Otherwise, better to leave a failure pin disconnected
