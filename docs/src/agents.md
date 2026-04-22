# Agents

- Language model with tools and context
- Memory can be provided as a tool
- In chat-driven paradigm, model must decide when to use tools
  - Must select from all available tools at a time
  - Limited context window on smaller models can cause confusion
- Workflow paradigm allows us to restrict tools and process results
- Can orchestrate multiple agents on subtasks

## Profiles & Roles

| A | B |
| -----| ----- |
| ![profile-free](./images/profile-free.png) | ![profile-local](./images/profile-local.png) |

Profiles are sets of models tagged with pre-defined roles.
Within a profile, each model can have multiple roles.
However, each role can be assigned to at most one model.

Profiles are user defined and the meaning of each is left for you to decide.
On the other hand, there are a fixed set of roles: reasoning, creative,
programming, etc. An agent can fill one of a fixed set of roles based on how it
will be used in the workflow.

These roles determine which model from the profile that the agent will use. If
a model works well on creative and reasoning tasks but has trouble generating
structured output, you can tag a different model with the `structured` role.

Together they provide a layer of abstraction that allows you to switch between
*local* vs *remote* providers during workflow authoring, for example. Then
during batch execution, you can supply a *production* profile. Alternatively,
you can compare how well a workflow runs with Anthropic and OpenAI models by
defining profiles for each without changing the workflow itself.

If a model is tagged with the *default* role it will be used in place of any
role that is not assigned to a model. For instance, if no model is tagged with
*creative*, when a *creative* agent runs, it will use the *default* model
instead. If no model is tagged with *default*, then the workflow will fail.
