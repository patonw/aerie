# Developers

## Setup

- Need a terminal (of course)
- If you already have a suitable build environment, try using it
- ...otherwise, [Install nix](https://nixos.org/download/)
  - Focus on reproducible declarative builds
  - Result should be the same on any machine
  - Drawback is that it needs to recreate build environment on every machine
- Recommend setting up [direnv](https://direnv.net/) also
- `git clone` the parent repository
- `cd refoliate/aerie`
- With direnv:
  - Optional: `echo "dotenv" >> ~/.envrc`
  - `echo "use nix" >> ~/.envrc`
  - `direnv allow`
  - Do not check in .env and .envrc files - ignored anyhow
  - Do not trust externally sourced .envrc files - security risk
  - Can add API keys and other environment variables to either .env or .envrc
  - Better to put them in .env to avoid reauthorizing after each edit
- without direnv:
  - Need to run `nix-shell` (no arguments) each time work with repo
  - Set environment variables manually or account-wide

## Build/Run

- `cargo run -- --session issue-####`
- Enable logging: `RUST_LOG=aerie=debug RUST_BACKTRACE=full cargo ...`
- Headless workflow run:
  ```bash
  cargo run --bin simple-runner -- \
    --temperature 0.3 \
    --model ollama/qwen3-coder:30b \
    --config <path-to-config> \
    tutorial/workflows/toolhead.yml
  ```
