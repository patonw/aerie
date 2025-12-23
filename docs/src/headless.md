# Workflow Execution

The main point of creating agentic workflows is to run them in automated tasks outside of the UI.

- Use runners
- Currently only simple-runner implemented
- Very limited functionality/customization
- Needs a better name
- Runs from the console using file inputs
- Can output to the console or disk
- Will not automatically load settings or session data
- Workflow must be a path to a workflow file, not just a name
- Only tool providers in config can be used
- If output directory specified
  - All outputs written to individual files
  - otherwise, will print to console in single JSON object
- To use tools you must specify a config file
  - Not loaded automatically
- Can run chained workflows using `autoruns` parameter
  - Must specify a workflow store with `workstore`
  - Workflows must call the chaining tool for successor
  - Each run will output a new JSON document in console mode
  - If outputting to directory, creates a subdir for each run

```console
$ simple-runner --help
A minimalist workflow runner that dumps outputs to the console as a JSON object.

If you need post-processing, use external tools like jq, sed and awk.

Usage: simple-runner [OPTIONS] <WORKFLOW>

Arguments:
  <WORKFLOW>
          The workflow file to run

Options:
  -w, --workstore <WORKSTORE>
          Path to a workflow directory for chain execution

  -o, --out-dir <OUT_DIR>
          Save outputs as individual files in a directory

  -s, --session <SESSION>
          A session to use in the workflow. Updates are discarded unless `--update` is also used

  -c, --config <CONFIG>
          Configuration file containing tool providers and default agent settings

  -b, --branch <BRANCH>
          The session branch to use

      --update
          Save updates to the session after running the workflow

  -m, --model <MODEL>
          The default model for the workflow. Has no effect on nodes that define a specific model

  -t, --temperature <TEMPERATURE>


  -p, --prompt <PROMPT>
          Initial user prompt if required by the workflow

  -a, --autoruns <AUTORUNS>
          Number of extra turns to run chained workflows

          [default: 0]
```

```console
$ simple-runner -- --temperature 0.3 --model ollama/qwen3-coder:30b --prompt "hmmm" --workstore ~/.local/share/aerie/workflows chaindrive --autoruns 3
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
