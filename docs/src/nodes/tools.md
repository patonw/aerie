# Tools

## Select Tools

- Provides a limited set of tools to the agent
- Best to only enable tools relevant to the subtask
- Can select entire providers[^select-provider] or individual tools

[^select-provider]: Selecting an entire provider also means that any tools added in future versions
are added automatically. If you individually select tools, future additions will not be included.

## Invoke Tool

- Only invokes one tool at a time currently
- Multiple/parallel invocations planned for future, gated by node parameter
- If only one tool available, `tool name` is optional
- arguments structure dictated by tool definition
- Will update chat history with results
- Since tools have no output schema
  - can't guarantee valid output
  - or even that it's JSON
  - Add Parse and Validate nodes as needed
