# Aerie

> [*A nest of a bird of prey perched high on a cliff or tree top.*](https://www.youtube.com/watch?v=B4-L2nfGcuE)

Aerie is a tool for building and running AI-powered workflows. Rather than giving
a language model free rein over a complex task, Aerie lets you break tasks into
discrete, well-defined steps — with AI playing a focused role in key steps. The
result is more predictable and debuggable than purely agent-driven systems.

![aerie-ui](./docs/src/images/showcase.jpg)

## Installation

Download the AppImage from the [releases page](https://github.com/patonw/aerie/releases):

```bash
chmod +rx aerie-x86_64.AppImage
./aerie-x86_64.AppImage
```

The AppImage also runs on Windows via
[WSL](https://learn.microsoft.com/en-us/windows/wsl/tutorials/gui-apps). For
Nix or source builds, see the [installation
guide](https://patonw.github.io/aerie/user_start.html#installation) and
[development guide](https://patonw.github.io/aerie/dev_start.html).

## Workflows

Workflows are structured as **node graphs**: each node can represent an agent,
data transformation, decision, or other action. Data flows in one direction
along wires connecting nodes, making execution order explicit and the overall
logic easy to follow and explain.

Each node executes at most once per run. The editor also supports
**incremental runs**, which re-execute only the nodes that have changed since
the last run. This makes iteration fast during development — tweak a prompt or
swap a model and only the affected portion of the graph reruns, leaving
expensive upstream steps untouched.

Nodes can be grouped into **subgraphs**, which appear as a single node in the
parent workflow, keeping complex graphs organized and readable. Subgraphs can
themselves contain subgraphs, allowing complex workflows to be built up from
well-defined, reusable pieces. A special iterative variant can apply a subgraph
across an entire list of inputs — see [Iteration](#iteration) below.

See the first steps tutorial [^first] for an introduction to building
workflows, and the subgraphs and iteration tutorial [^iteration] for a deeper look.

[^first]: [first steps](https://github.com/patonw/articles/blob/main/aerie/beginner/01-first-flight.md)
[^iteration]: coming soon

> [!note]
> Tutorials are currently in progress. Links will be updated as they are published

Furthermore, workflows can dispatch to other workflows in a sequential chain,
enabling dynamic routing and tail recursion.

## Key Features

### Structured Data Generation and Extraction

Aerie's *Structured Output* node instructs an LLM to produce output conforming
to a [JSON Schema](https://json-schema.org/), making it straightforward to
extract structured data from natural language or generate it from scratch. Once
you have structured data, transformation and templating nodes let you reshape
and render it without touching the LLM again — keeping things cheap, fast, and
precise. See the structured generation tutorial [^structured].

[^structured]: coming soon

### Tool Integration via MCP

Agents can interact with external services through the [Model Context
Protocol](https://modelcontextprotocol.io/). Aerie manages MCP tool providers
from a dedicated Tools tab, supporting both local STDIO servers and remote HTTP
services. Tools can be selected per-agent, so each step in a workflow only has
access to what it needs. The agent tools tutorial [^agents] walks through a practical
example using live weather data.

[^agents]: coming soon

You can also use the *Invoke Tool* node to manually make tool calls — bypassing
LLM tool selection. This can be useful in cases where the arguments are known
ahead of time or need to be tightly controlled. It also allows for manipulating
the tool results before sending them to a language model. This is covered in
the tool invocation tutorial [^invocation].

[^invocation]: coming soon

### Iteration

Iterative subgraphs apply a nested workflow to every item in a list, collecting
the results into an output list. This makes it possible to process inputs
rigorously at scale — for instance, checking each extracted claim in a document
individually rather than asking the model to handle them all at once. Optional
parallelism can speed things up, though rate limiting is advisable with remote
APIs. See the subgraphs and iteration tutorial [^iteration].

### Batch Processing

Workflows don't have to be simple chat agents. Named *Output* nodes emit
results that can be consumed by other applications — written to the console or
individual files when run via the runner CLI:

```bash
aerie-runner \
    --workflows ~/.local/share/aerie/workflows/ \
    --model openrouter/openrouter/free \
    -I article.txt \
    exec my-workflow
```

See the [outputs tutorial] for details.

### Visual QA & Extraction

*Coming soon*...

Workflows can accept image inputs along with text to handle tasks like visual
question answering or converting images into structured data.

## Getting Started

Consult the [documentation](https://patonw.github.io/aerie/user_start.html) for
detailed instructions.

Import and experiment with the [Example Workflows](./examples/workflows).

The [tutorial series] builds up from a simple two-agent chat
workflow through structured generation, tool use, document analysis, iteration,
and batch output — each article building on the last. Start with First
Steps[^first].

## Project Status

Aerie is still under active development. While it may not be ready for
production use, it is well-suited for exploration and prototyping.

## Links

- [Source code](https://github.com/patonw/aerie)
- [Releases](https://github.com/patonw/aerie/releases)
- [User guide](https://patonw.github.io/aerie/user_start.html)
- [Development guide](https://patonw.github.io/aerie/dev_start.html)
- [Example Workflows](./examples/workflows)

## License

All projects in this repository are licensed under the
[Mozilla Public License Version 2.0](https://www.mozilla.org/en-US/MPL/2.0/)
