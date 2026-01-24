# JSON

## Parse JSON

- Attempt to convert raw text into a JSON value
- If unable to parse, outputs to failure pin
- Does not guarantee a particular structure
  - Result can be an object, array or primitive value
  - Use [Validate JSON](#validate-json) to ensure structure

## Gather JSON

- Takes JSON or primitive values into a JSON array
- Does not [parse text into JSON](#parse-json)
- Can take any number of inputs
- Input pins correspond to index in array
- Unwired pins will have null entries in the output
- Rarely useful alone, but works well with [Transform JSON](#transform-json)

## Validate JSON

- Uses a [JSON schema](https://tour.json-schema.org/) to ensure the structure of a JSON value
- You can use an LLM to generate a schema
- Can also create from examples using generators on the web:
  - <https://app.quicktype.io/>
  - <https://www.jsonforge.com/tools/schema-generator>
  - <https://jsonutils.org/json-schema-generator.html>

## Transform JSON

- An advanced tool for manipulating JSON documents
- Uses [jaq](https://gedenkt.at/jaq/manual/) which is based on [jq](https://jqlang.org/)
  - Functional expression language for structural transformations
  - Very flexible and efficient once familiar
- Use cases
  - Restructure arrays from [Gather JSON](gather-json) into objects
  - Transform JSON value into arguments for [Invoke Tool](tools.md#invoke-tool)
  - Create templating context for [Template](value.md#template) nodes
  - Filter and restructure data from tool results

## Unwrap JSON

- Convert a JSON value into a native wire type
- If JSON input is not compatible, node will emit a failure
- Use Parse/Transform/Unwrap to extract data from (semi-) structured text
  - e.g. parse a tool result, transform to a single value, then unwrap
