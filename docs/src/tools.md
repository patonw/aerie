# Tools

- Allow your agents to interact with the outside world
- Can retrieve information from the web or private data stores
  - web search using public APIs
  - memorization and recall
  - check the weather forecast
  - semantic search a vector store
- Some can execute actions, e.g.
  - Send messages over a chat service
  - running software on your computer
  - approving/banning content in a moderation API
  - ordering parts and supplies
- Anything that can be done by software can be turned into a tool
- Recently tools standardized around Model Context Protocol (MCP)

## Providers

- Tools are supplied by tool providers
- MCP providers can run locally or remotely
- Local providers can perform actions on your computer
- External providers can integrate with various web services

## Configuration

- Add various providers in the Tools tab
- Saved to configuration file
- Remote providers may require authorization
  - Can specify an environment variable containing auth token
- Can specify timeout

### Examples

#### MCP example (server-everything)

- A bit useless aside from verifying setup works
- Create a new STDIO tool
  - command: `nix-shell`
  - Arguments[^newlines]:
    ```shell
    -p
    nodejs_22
    --run
    npx -y @modelcontextprotocol/server-everything
    ```
- Peruse the tool schemas
- Try using [Parse JSON](./nodes/json.md#parse-json) and [Invoke Tool](./nodes/tools.md#invoke-tool) to call these manually
- You can use an agent with [Structured Output](./nodes/agent.md#structured-output) to help with the arguments

#### Tavily

- Web search API geared optimized for agents
- 1000 free requests / month
- Can run MCP server locally or use hosted
- First sign up and get an API key from [Tavily](https://www.tavily.com/)
- Define an environment variable for the API key
  - `TAVILY_API_KEY=***`
- Start the app and define a new HTTP tool
  - URI: `https://mcp.tavily.com/mcp/?tavilyApiKey={{api_key}}`
  - Auth Var: `TAVILY_API_KEY`
- App substitutes `{{api_key}}` when issuing requests

#### EmbCP

- An example MCP server on top of [Qdrant](https://qdrant.tech/) included in this repo
- You'll need to index something yourself to get this running
- first run a qdrant server and create a collection
  - Included helper script: `nix-shell . --run qdrant-serve`
- Create collection and import some example points using emberlain
  ```sh
  cd emberlain
  cargo run -- \
    --progress \
    --llm-model my-qwen3-coder:30b \
    --embed-model MxbaiEmbedLargeV1Q \
    --collection goose \
    /code/upstream/goose/crates/goose/src/agents/
  ```
  - You'll need to select a suitable provider and model for your setup
  - Select an embedding model that performs best for your case
  - Also supply a code path that exists on your system
  - This can take a while and/or burn through credits, depending on your provider
  - Import is incremental
    - you can interrupt and resume
    - or index parts of the repo selectively
- Install embcp: `nix-env --install -f . -A embcp-server.bin`
- Define a new stdio tool
  - Command: `embcp-server`
  - Arguments[^newlines]:
    ```sh
    --embed-model
    MxbaiEmbedLargeV1Q
    --collection
    goose
    ```
    - Must use same embedding model and collection as previous step
    - Qdrant serve must still be running independently

[^newlines]: Placement of lines matters here. Each argument to the defined Command should be on a separate line. Multiple words on a single line will be treated as a single argument.

## In workflows

- In workflows, can restrict which tools a given agent can use
- Can also invoke tools manually for more control over inputs and results
- Tools will automatically be used by the Chat Node if LLM opts to use
- In Structured Chat, a tool must be used
- Tool invocation can fail for numerous reasons
  - Can use Fallback node to recover
- External tools may fail to respond in a reasonable time
  - Set the timeout in the provider configuration
