# Users

## Installation

- Linux AppImage binary builds available under [releases](https://github.com/patonw/refoliate/releases)
  - Download from Assets
  - Runs on most common distributions
  - Can run on Windows under [WSL](https://learn.microsoft.com/en-us/windows/wsl/tutorials/gui-apps)
  - On NixOS requires nixGL and appimage-run
- Also available as source
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
- API keys can be supplied by environment variable
  - Common practice and convenient
  - specific method to set variables depends on platform
  - Not particularly secure
  - Account-wide variables visible to unrelated apps
  - Anyone can inspect variables of running apps
- Ephemeral file handles more secure
  - Contents still dotenv format
  - Not perfect but far more secure
  - Avoid persistent files on disk (still allowed, however)
  - Use external vault or password manager to store secrets
  - When launching app, use process substitution or stdio pipe
- Variable name depends on provider
- Some providers require additional settings like API host or base
- Refer to rig docs for more details

Example of passing secrets through file handle from Bitwarden:

```bash
$ bw unlock
# ... login steps ...
$ export BW_SESSION=***
$ aerie --env <(bw get notes "API Keys") ...
```

Where "API Keys" is a secure note that looks like:
```env
OPENROUTER_API_KEY=***
MISTRAL_API_KEY=***
TAVILY_API_KEY=***
...
```

> 📢 Avoid exposing the literal values of the API keys on the command line.
>
> **Don't do this:** `echo "SOME_API_KEY=asdf" | aerie --env -`
>
> The API key will be saved to your history and visible from the process list.
> Use a plain text file, instead, when security is *not* critical.
> It will be easier to restrict or redact after the fact.

### Provider Examples

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

> 📢 You must use `ollama pull` to download any models before using them

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
