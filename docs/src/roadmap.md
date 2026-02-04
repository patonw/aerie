# Roadmap

- [ ] Documentation to actual sentences
- [x] Deprecate Model values and replace instances with Text
  - Switch text storage to `smol_str`
- [ ] Warn on missing tools & providers in mentioned in selection
- [x] Extensibility
  - make crate usable as a library
  - Box dyn nodes
  - Runtime registration
- [x] Subgraphs
  - Subgraph defined by ~storage and~ flavor (e.g. simple, retry, match)
  - ~Storage can be Inline or Named~
    - Inline subgraphs edited inside node
    - ~Named subgraphs can only be edited from manager~
    - ~Can convert between inline and named within a root graph~
    - ~Named subgraphs cannot be referenced by another subgraph~
    - ~i.e. only top-level~
    - Subgraphs can contain nested inline subgraphs
- [x] rhai script node
- [ ] WASM plugins
- [ ] Media support
  - Image inputs first
  - Input collection management
  - ~input file args in runner~
  - support file names and data urls

## Low Priority

- [ ] Tab layout persistence
- [ ] Credential helpers
  - Encrypted environment variables
  - Only unlocked in memory
  - Prompt for passphrase
  - Can we just leverage lastpass, bitwarden, etc?
  - How about dbus secrets management?
- [ ] Runner parallelism using rayon on ready nodes
- [ ] Data parallelism
  - Implement nodes for filter, partition, group etc
- [ ] Concurrent LLM calls
  - Need to throttle by provider (use separate pools?)
- [ ] Sharing/publishing via web
  - [ ] Import/Export root graphs/subgraphs
