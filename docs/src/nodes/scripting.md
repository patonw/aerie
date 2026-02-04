# Scripting

## Rhai

- Interprets a [Rhai script](https://rhai.rs/book/index.html) on inputs to produce outputs
- Useful for generating/transforming complex data
  - Overlaps with [Transform JSON](./json.md#transform-json) but uses general purpose language
  - Both can split/merge/reorder/etc. lists and objects
  - Also can be used to replace more specific nodes:
    - number and text values
    - templating
    - gathering/unwrapping JSON
  - More convenient for literal lists than parse & unwrap JSON
- Inputs injected into script by pin name
  - non-alphanumeric characters replaced by underscore
- Outputs extracted from evaluation result of script
  - Final expression
  - If only one output, the entire result is emitted on wire
  - If multiple outputs and result is an array
    - elements are matched by output pins by index
  - If multiple outputs and result is a dictionary
    - Output names matched to dictionary keys
