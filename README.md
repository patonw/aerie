# Aerie

> [*A nest of a bird of prey perched high on a cliff or tree top.*](https://www.youtube.com/watch?v=B4-L2nfGcuE)

Aerie is a graphical tool for building AI-powered workflows. Rather than giving
a language model free rein over a complex task, Aerie lets you break tasks into
discrete, well-defined steps — with AI playing a focused role in each. The
result is more predictable and debuggable than purely agent-driven systems.

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

Workflows are structured as **node graphs**: each node represents an agent,
data transformation, decision, or action. Data flows in one direction along
wires connecting nodes, making execution order explicit and the overall logic
easy to follow and explain.

Each node executes at most once per full run. The editor also supports
**incremental runs**, which re-execute only the nodes that have changed since
the last run. This makes iteration fast during development — tweak a prompt or
swap a model and only the affected portion of the graph reruns, leaving
expensive upstream steps untouched.

Nodes can be grouped into **subgraphs**, which appear as a single node in the
parent workflow, keeping complex graphs organized and readable. Subgraphs can
themselves contain subgraphs, allowing complex workflows to be built up from
well-defined, reusable pieces. A special iterative variant can apply a subgraph
across an entire list of inputs — see [Iteration](#iteration) below.

See the [first steps tutorial](./tutorials/01-first-steps.md) for an
introduction to building workflows, and the [subgraphs and iteration
tutorial](./tutorials/11-iteration.md) for a deeper look.

## Key Features

### Structured Data Generation and Extraction

Aerie's *Structured* node instructs an LLM to produce output conforming to a
[JSON Schema](https://json-schema.org/), making it straightforward to extract
structured data from natural language or generate it from scratch. Once you
have structured data, transformation and templating nodes let you reshape and
render it without touching the LLM again — keeping things cheap, fast, and
precise. See the [structured generation
tutorial](./tutorials/02-structured.md).

### Tool Integration via MCP

Agents can interact with external services through the [Model Context
Protocol](https://modelcontextprotocol.io/). Aerie manages MCP tool providers
from a dedicated Tools tab, supporting both local STDIO servers and remote HTTP
services. Tools can be selected per-agent, so each step in a workflow only has
access to what it needs. The [agent tools
tutorial](./tutorials/03-agent-tools.md) walks through a practical example
using live weather data.

Workflows can also invoke tools directly — bypassing LLM tool selection — for
cases where the arguments are known ahead of time or need to be tightly
controlled. This is covered in the [tool invocation
tutorial](./tutorials/13-tool-use.md).

### Iteration

Iterative subgraphs apply a nested workflow to every item in a list, collecting
the results into an output list. This makes it possible to process inputs
rigorously at scale — for instance, checking each extracted claim in a document
individually rather than asking the model to handle them all at once. Optional
parallelism can speed things up, though rate limiting is advisable with remote
APIs. See the [subgraphs and iteration tutorial](./tutorials/11-iteration.md).

### Batch Processing

Workflows don't have to be chat agents. Named output nodes emit results that
can be consumed by other applications — written to the console or individual
files when run via the `simple-runner` CLI:

```bash
aerie-runner \
    -w ~/.local/share/aerie/workflows/ \
    -m openrouter/openrouter/free \
    -I article.txt \
    my-workflow
```

See the [outputs tutorial](./tutorials/12-outputs.md) for details.

## Getting Started

The [tutorial series](./tutorials/) builds up from a simple two-agent chat
workflow through structured generation, tool use, document analysis, iteration,
and batch output — each article building on the last. Start with [First
Steps](./tutorials/01-first-steps.md).

## Links

- [Source code](https://github.com/patonw/aerie)
- [Releases](https://github.com/patonw/aerie/releases)
- [User guide](https://patonw.github.io/aerie/user_start.html)
- [Development guide](https://patonw.github.io/aerie/dev_start.html)
- [Model Context Protocol](https://modelcontextprotocol.io/)

## License

All projects in this repository are licensed under the
[Mozilla Public License Version 2.0](https://www.mozilla.org/en-US/MPL/2.0/)
