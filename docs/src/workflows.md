# Workflows

- Represents a task to be completed by one or more agents
- Consists of a set of nodes
  - Smallest unit of work in the workflow
- Nodes have inputs and outputs
  - Input pins can accept various types depending on their role
  - Output pins have only one concrete type (at a time)
    - Some nodes have dynamic outputs that are determined by the input wire
    - They still only have a single type, but it can change
- Wires connect node outputs to other node inputs [^cycle]
  - Each wire has a specific type (text, agent, conversation, etc)
  - Type determined by the output pin
- Each node has its own set of parameters
  - Parameters can be supplied through controls on the node
  - Some can be supplied by input
- A node will produce outputs when run
  - Output values will be determined by inputs and parameters
  - Global application state only comes into play at Start and Finish nodes
  - State of external tools and models might affect outcome
  - If LLM providers could honor PRNG seed, results would be reproducible
    - Many accept it in API, but still have non-deterministic results
- Workflows can contain [subgraphs](./subgraphs.md)

[^cycle]: Creating circular connections is possible (for now), but any nodes in the cycle will never run.

## Execution model

- Nodes are run as they become ready
- Readiness is determined by whether a node is waiting for inputs
- If inputs supplied by another node:
  - when other is waiting or running, the node is not ready
  - when all others complete, node becomes ready
  - Exception: [Select](./nodes/control.md#select) takes first ready input
  - if other node fails, node will never become ready
  - ...unless, it is attached as a failure handler
- If a node fails without failure handlers, workflow stops
  - handlers attached by routing failure pin to another node's input
  - Handler can be anything that accepts a failure
  - However, Fallback node is usually the most sensible
  - Other options: Output, Gather and Preview to squelch errors
- When a node completes, it supplies values to all output pins
  - Exception: [Match](./nodes/control.md#match) only sends output to pin matching key
- Nodes currently run one at a time
  - Concurrency will be implemented in the future
  - At the moment, if multiple nodes ready, highest priority executed first
  - Priority defined by node implementation
  - Can use the Demote node to locally adjust priority
    - Only affects immediate successor
- Only one Start/Finish node per workflow
  - If Finish node is connected, it must run or the execution will fail
  - Use [Select](./nodes/control.md#select) to join diverging branches into one value
  - Use [Select](./nodes/control.md#select) with [Demote](./nodes/control.md#demote) to provide default values

## Workflow input

- Workflow input is a raw string
- In the UI this is taken from the prompt box of the chat tab
- You can treat this however you wish
- Parsing it as JSON can be useful in many situations
- A workflow can have an input schema
- Can edit from the same box as description
- Schema will be emitted from the Start node

## Workflow Outputs

- Documents produced as byproduct of workflow run [^inputs]
- Not to be confused with node outputs
- Not part of the session
- Can result from tool calls or chat nodes through intermediate processing
- Cannot be emitted from subgraphs, only top-level workflows

[^inputs]: Loading local documents directly into workflow not supported yet, but can be done with tools

## Chain execution

- Workflows can be set to run automatically in a sequence
- One workflow calls a chain tool to queue its successor
- If the autorun setting is enabled, the next workflow will run automatically
- Otherwise, you can start the next run manually
- Runner will output a separate JSON document for each run
  - This is not a JSON array, but a stream of individual documents
  - Use `jq -s` to convert it into an array if needed
- Chain tools available from the Tool Selector node
  - Only when enabled in the chain tab of the workflow metadata
  - Chain tool selection not available inside subgraphs
  - Can be passed from root workflow to subgraphs however
  - Can use structured prompts to pass data
  - If schema defined on target workflow it is used in tool definition
- `autoruns` setting allow UI to automatically run a chain
- Tool can be called by LLM or set manually
- If you exhaust the autorun count and want to continue
  - in the UI simply use the Run or chat buttons to start a new execution
  - From a runner, start your initial run with `--next`
  - This will output an additional JSON object with the next workflow to run
