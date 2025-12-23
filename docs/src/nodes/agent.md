# LLM/Agent

These nodes are specifically for interacting with external language models.

## Agent

- Configuration of a language model with available tools
- Can create an agent from scratch or modify a previous agent
- Tools supplied from a [Select Tools](./tools.md#select-tools) node
- Can provide a system message
  - Provide instructions or hints about agent's style, perspective or personality
  - Should not by used to inject [context](#context)

## Context

- Manually injects context into an agent
- This is a document that an agent can refer to when addressing a user prompt
- Handled separately from tool call results, which can be used similarly
- Some models might perform better using one or the other

## Chat

- Produces an unstructured response as its final output
- When given tools, it may take additional turns internally
- Intermediate turns are recorded in the conversation
- Can fail if the provider is unavailable or the model id is invalid

## Structured Output

- Produces structured responses as JSON values
- Two main use cases:
  - Forcing a tool call and getting the parameters
  - Producing documents with a specific structure using a [schema](https://json-schema.org/understanding-json-schema/reference)
- Essentially the same thing to the model
- Difference is what we do with the result
- When using Structure Output to generate a tool call, the tool is not invoked automatically
  - Must use [Invoke Tools](./tools.md#invoke-tools)
  - Allows you to modify the parameters
  - The `extract` option can work around common failure modes of smaller language models
  - Generally safe in this circumstance since only a small number of JSON-like substrings
- [JSON schemas](https://json-schema.org/understanding-json-schema/reference) can be as permissive or specific as desired
  - Chat with an LLM to help develop one by supplying it with examples and constraints
  - You can also use schema generators online
