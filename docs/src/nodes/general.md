# General

These nodes provide scaffolding for the workflow.

## Start

- Gathers global state and injects it into workflow
- Exposes [settings](../interface.md#settings) to workflow
- Usually first node run in any workflow
- Other nodes without inputs can run before

## Finish

- May or may not be last node run
- Returns data back to the global state
- The conversation must be an extension of the input
- Other nodes may continue to run after Finish if not on its path

## Preview

- Transient display for wire values
- Has no external effect
  - Except as a failure handler
  - Will swallow errors and display them on canvas
- Values are not persisted

## Output

- Emits documents as a result of running the workflow
- In the UI, listed in the Outputs tab
- Must be saved individually
- Runner can print to console or save to disk

## Comment

- No functionality
- Only for documentation and communication
