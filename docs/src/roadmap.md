# Roadmap

- [ ] Documentation to actual sentences
- [ ] Warn on missing tools & providers in mentioned in selection
- [ ] `runner serve` subcommand
  - Local HTTP server with /exec endpoint
  - exec args as posted JSON
  - Avoids startup latency during multiple runs
- [ ] `runner mcp [stdio|http]` subcommands
  - MCP server over STDIO and HTTP
  - Workflows as tools
    - Expose description and schema
    - payload param for workflow input
  - top-level params for conversation, autorun, temperature, model
- [ ] `ui` feature flag
  - exclude egui & other deps when disabled
  - headless workflow node impls only

## Low Priority

- [ ] WASM plugins
- [ ] Tab layout persistence
- [ ] Credential helpers
  - Encrypted environment variables
  - Only unlocked in memory
  - Prompt for passphrase
  - Can we just leverage lastpass, bitwarden, etc?
  - How about dbus secrets management?
- [ ] Runner parallelism using rayon on ready nodes
- [ ] Concurrent LLM calls
  - Need to throttle by provider (use separate pools?)
- [ ] Sharing/publishing via web
  - [ ] Import/Export root graphs/subgraphs
- [ ] Client-server split for UI
  - [ ] optional web client
