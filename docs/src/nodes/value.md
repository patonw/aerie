# Value

## Number

- A simple integer or floating point number
- Non-functional input allows it to be part of control flow
- e.g. can be placed between [Fallback](./control.md#fallback) and [Select](./control.md#select)

## Plain Text

A node to create text values without structure or formatting. Can convert some
values (e.g. messages) to plain text.

Optionally, larger text values can be
split into text lists using various delimiters.

## Template

- Uses [minijinja templates](https://docs.rs/minijinja/latest/minijinja/syntax/index.html) to convert a context into plain text
- variables input is a JSON object
  - Typically supplied by [Structured Output](agent.md#structured-output) or [Invoke Tools](tools.md#invoke-tools) with [Parse JSON](json.md#parse-json)
  - Can also be constructed by using [Transform JSON](json.md#transform-json) paired with [Gather JSON](json.md#gather-json)
  - Scalar inputs will be wrapped with the key "value"
  - You can access the entire input as `CONTEXT` inside a template
    ```jinja
    Hello, your input was:
    {{ CONTEXT | tojson(indent=2) }}

    Have a nice run.
    ```
- Use cases
  - Formatting human readable reports from structured data or tool results
  - Transforming data into formats that a language model can digest more easily
  - Combine data from multiple paths in the graph
