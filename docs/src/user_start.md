# Users

## Installation

- Currently only available as source
  - Only tested on Linux
  - Process is fairly automated once build tool installed
- Need a terminal
- [Install nix](https://nixos.org/download/)
  - A package manager and build tool
  - Can temporarily or permanently fetch multitude of utilities and apps
  - Might need to add ~/.nix-profile/share to `XDG_DATA_DIRS`
- Temporary git:
  - `nix-shell -p git`
- Get parent repo sources:
  - `git clone --recurse-submodules https://github.com/patonw/refoliate`
  - `cd refoliate`
  - `git submodule update --init --recursive` (only needed for updates)
- Install using nix-env:
  - `nix-env --install -f . -A aerie.app`
  - this takes a while the first time: must create a build environment
    - More of a lunch break than a coffee break
  - Can launch from command line or desktop environment
  - Some desktops may require logging out to refresh app list

## First steps

- Opens in with Chat tab and settings sidebar
- If send a prompt now, nothing happens
  - No LLM provider configured
- Switch to the Workflow tab
- In the Workflow drop-down, select basic
  - Doesn't do anything useful, but has many notes
  - Pay attention to the description box at the bottom left
  - Can be scrolled and resized
  - Click on "Run" to run the workflow
  - Outputs appear in the output tab of the side panel
  - Can view or save outputs
- From the workflow drop-down select chatty
  - May look familiar:
    - it's the same as the default workflow but with a lot of comments
  - You can edit this to customize simple chatting
    - Can also edit the default flow, but changes are not persistent
  - Running this should produce an error since provider is not configured

## LLM Providers

- Provider is specified in the prefix of the model
- e.g. `openai/gpt-4o` will connect to OpenAI API
- API keys and must be supplied by environment variable
- No way to configure in the app. Bad practice regardless.
- Environment variable depends on provider
- Method to set variables depends on environment
- Some providers require additional settings like API host or base
- Refer to rig docs for more details

### Examples

#### OpenRouter

- Environment:
  - `OPENROUTER_API_KEY=sk-********`
- model key:
  - `openrouter/mistralai/devstral-2512:free`

#### Mistral

- Environment:
  - `MISTRAL_API_KEY=********`
- model key:
  - `mistral/labs-devstral-small-2512`

#### Ollama

You typically won't be setting an API key for ollama, since you'll be managing this provider yourself. If you're running ollama on the same machine with default port, you probably won't need to set the API base URL. This is needed for instance if you're running ollama on your desktop and working off your laptop or you have a headless deep learning server separate from your primary computer.

- Environment:
  - `OLLAMA_API_BASE_URL=http://10.11.12.13:11434`
- model key:
  - `ollama/qwen3-coder:30b`

> [!IMPORTANT]
> You must use `ollama pull` to download any models before using them

## Cleanup

- Uninstall via nix-env:
  - `nix-env --uninstall aerie`
- Sessions/workflows
  - ~/.local/share/aerie/sessions
  - ~/.local/share/aerie/workflows
  - ~/.local/share/aerie/backups
- Configuration files
  ~/.config/aerie/*
- [Uninstall nix](https://nix.dev/manual/nix/2.33/installation/uninstall.html)
