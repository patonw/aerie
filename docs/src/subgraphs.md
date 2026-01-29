# Subgraphs

- Workflows can be nested inside the [Subgraph](./nodes/general.md#subgraph) node
- Customizable inputs/outputs
- Output nodes are disabled inside subgraphs
  - Partially to eliminate confusion of outputs hidden deep in subgraph hierarchies
  - Behavior would be undefined when iterative subgraphs are implemented
- Subgraphs can only trigger chaining with tools passed as input from top-level
  - Chain execution not available on Tool nodes inside subgraph
- Failures in a subgraph always captured by the failure pin
  - Regardless of how the failure would be handled at the node level

## Simple Subgraphs

- Use cases:
  - Organize a complex workflow into logical units (i.e. refactoring)
  - Ensure a group of related nodes runs together without interruption
  - Group useful reusable blocks into library workflows to publish/share
    - Copy/paste for to import into actual workflows
- Run at most once per workflow execution
- If one or more inputs never ready, then subgraph will not run
- Wires must exactly match the input pin type
- The entire subgraph runs from start to finish as the node is evaluated
  - Execution of nodes inside the subgraph are not interleaved with parent nodes
  - i.e. Priority of parent nodes irrelevant during subgraph execution
- Inputs from diverging branches
  - A subgraph taking inputs from diverging branches will never run
  - e.g. different cases on a match or success and failure from a falliable node
  - Either use different subgraphs for each branch
  - Or include the divergence inside the graph
- Diverging outputs
  - A subgraph will diverging outputs will start but never finish
  - All outputs inside the subgraph must be ready for a subgraph to finish
  - Use Select nodes to pick outputs or defaults (when combined with Demote)

## Iterative Subgraphs

- Use cases:
  - Chunk one text into a text list then insert each into a vector database
  - Generate multiple candidate queries/prompt and run a workflow on each
  - Summarize each section/paragraph of a document then reducing to a meta-summary
- May run multiple times per workflow execution
- Once per element of list valued inputs
  - A single Subgraph node runs its iterations in parallel
  - Different nodes, even with same inputs, do not run in parallel (yet)
  - Caution: no rate-limiting is used so API limits can cause failures
- Node will expose a list variant for each of the following types on its Start node:
  - Text -> TextList
  - Integer -> IntList
  - Number -> FloatList
  - Message -> MsgList
  - JSON -> JSON (but as an Array value)
- List inputs can be attached to list or scalar wires
- List input must have the same number of elements
- Scalar inputs will be broadcast (repeated) each run
- Scalar values on Finish node will also be translated to list outputs
- List values on Finish node will be flattened
  - Nesting lists is not possible
  - Example use cases:
    - concatenating results of multiple variations of a search query
  - Can also filter inputs by emitting empty list for some runs
