# Workflow Execution

The main point of creating agentic workflows is to run them in automated tasks outside of the UI.

- Execute through the runner utility
- Can take string or file inputs
- Can output to the console or disk
- supports tools, sessions and chaining
- If session loaded can also be updated or not
- Workflow can be a file path
  - Can also use workflow directory in addition to a name
- Can use tools if given directory to provider definitions
- If output directory specified
  - All outputs written to individual files
  - otherwise, will print to console in single JSON object
- Can run chained workflows using `autoruns` parameter
  - Must specify a workflow store with `workstore`
  - Workflows must call the chaining tool for successor
  - Each run will output a new JSON document in console mode
  - If outputting to directory, creates a subdir for each run

```console
$ aerie-runner --help
A minimalist workflow runner that dumps outputs to the console as a JSON object.

If you need post-processing, use external tools like jq, sed and awk.

Usage: aerie-runner [OPTIONS] <COMMAND>

Commands:
  exec  
  help  Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>
          Configuration file containing tool providers and default agent settings

  -e, --env <ENV>
          An ephemeral file handle to dotenv formatted secrets

  -w, --workflows <WORKFLOWS>
          Directory containing workflows

  -t, --tools <TOOLS>
          Directory containing tool provider definitions

  -s, --session <SESSION>
          A session to use in the workflow. Updates are discarded unless `--update` is also used

  -b, --branch <BRANCH>
          The session branch to use

      --update-session
          Save updates to the session after running the workflow

  -m, --model <MODEL>
          The default model for the workflow. Has no effect on nodes that define a specific model

  -T, --temperature <TEMPERATURE>
          Default language model temperature

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version

$ aerie-runner help exec
Usage: aerie-runner exec [OPTIONS] <WORKFLOW>

Arguments:
  <WORKFLOW>  The workflow file to run

Options:
  -i, --input <INPUT>            Initial user prompt if required by the workflow [aliases: --prompt]
  -I, --input-file <INPUT_FILE>  Path to file containing the initial prompt
  -o, --out-dir <OUT_DIR>        Save outputs as individual files in a directory
  -a, --autoruns <AUTORUNS>      Number of extra turns to run chained workflows [default: 0]
  -n, --show-next                Prints an additional object containing the next workflow after the last run
  -h, --help                     Print help
```

```console
$ aerie-runner -- -T 0.3 -m ollama/qwen3-coder:30b -w ~/.local/share/aerie/workflows exec drivechain --prompt "hmmm" --autoruns 3
{
  "prompt": "hmmm"
}
{
  "prompt": "hmmm\n\nHello, again!"
}
{
  "prompt": "hmmm\n\nHello, again!\n\nHello, again!"
}
{
  "prompt": "hmmm\n\nHello, again!\n\nHello, again!\n\nHello, again!"
}
```
