# Request scripts

Test scripts for HTTP/SSE runners.

## Start server

Before making any requests, ensure the server is running.

To run against the examples in this project using your current environment:

```bash
RUST_LOG=aerie=info,runner=info \
  cargo run --release --bin runner -- \
  -w examples/workflows/intermediate \
  -t examples/tools/nix \
  serve
```

Workflows using agents and tools may require additional settings and/or environment.
To start a server that uses the resources of the GUI client:

```bash
RUST_LOG=aerie=info,runner=info \
  cargo run --release --bin runner -- \
  -c ~/.config/aerie/settings.toml \
  -w ~/.local/share/aerie/workflows \
  -t ~/.local/share/aerie/tools/ \
  --env OPTIONAL_DOT_ENV_FILE_OR_PIPE \
  serve
```

## Invoke requests

Invoke from the project root passing in an environment name:

```bash
$ scripts/requests/loopy foobar
HTTP/1.1 200 OK
Cache-Control: no-cache
Content-Type: text/event-stream
Date: Mon, 13 Jul 2026 19:17:38 GMT
Transfer-Encoding: chunked

: Switching to new output channel

event: run-event
data: {"tags":["/Preview.preview"],"data":{"Integer":21},"#clk_ms":19}

...

event: output
data: {"gcd":{"Integer":15}}

event: run-event
data: {"tags":["/Preview.preview"],"data":{"Text":"foorenbar"},"#clk_ms":33}

: Finished task in 0.03s with result: Ok(Ok(()))
```

You can pass the `--offline` option to see the request headers and parameters
instead of sending the request.

```bash
$ scripts/requests/loopy foobar --offline
POST /sse HTTP/1.1
Accept: application/json, */*;q=0.5
Accept-Encoding: gzip, deflate, br, zstd
Connection: keep-alive
Content-Length: 47
Content-Type: application/json
Host: localhost:8058
User-Agent: xh/0.25.3
X-User-Env: OMDB_API_KEY=placeholder
X-User-Env: WHOAMI=foobar
X-User-Name: foobar

{
    "workflow": "loopy",
    "#events": [
        "**/*.preview"
    ]
}
```

To add/override parameters, simply pass additional
[httpie style options](https://github.com/ducaale/xh#request-items)
as arguments:

```bash
scripts/requests/loopy blank X-Forwarded-For:127.0.0.1 '#events[0]=**'
```

This adds an additional header and overrides the event filter
to include all events.
