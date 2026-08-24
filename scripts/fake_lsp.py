#!/usr/bin/env python3
"""A minimal scripted language server for protocol tests.

Speaks just enough LSP over stdio to exercise Textchum's client end to
end: answers `initialize`, and replies to every `textDocument/didOpen` and
`textDocument/didChange` with one canned diagnostic on the first line
whose message includes the request count — so tests can assert both
delivery and freshness. Exits cleanly on `shutdown`/`exit`.
"""

import json
import sys


def read_message(stream):
    length = None
    while True:
        line = stream.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1])
    if length is None:
        return None
    return json.loads(stream.read(length))


def send(payload):
    body = json.dumps(payload).encode("utf-8")
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()


def main():
    stdin = sys.stdin.buffer
    seen = 0
    while True:
        message = read_message(stdin)
        if message is None:
            return
        method = message.get("method")
        if method == "initialize":
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {"capabilities": {"textDocumentSync": 1}},
            })
        elif method in ("textDocument/didOpen", "textDocument/didChange"):
            seen += 1
            uri = message["params"]["textDocument"]["uri"]
            send({
                "jsonrpc": "2.0",
                "method": "textDocument/publishDiagnostics",
                "params": {
                    "uri": uri,
                    "diagnostics": [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 4},
                        },
                        "severity": 1,
                        "message": "fake finding #%d" % seen,
                    }],
                },
            })
        elif method == "textDocument/hover":
            position = message["params"]["position"]
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "contents": {
                        "kind": "markdown",
                        "value": "fake hover at %d:%d"
                        % (position["line"], position["character"]),
                    }
                },
            })
        elif method == "textDocument/definition":
            uri = message["params"]["textDocument"]["uri"]
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": [{
                    "uri": uri,
                    "range": {
                        "start": {"line": 0, "character": 3},
                        "end": {"line": 0, "character": 7},
                    },
                }],
            })
        elif method == "textDocument/completion":
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "isIncomplete": False,
                    "items": [
                        {"label": "fake_function", "kind": 3,
                         "detail": "fn fake_function()",
                         "insertText": "fake_function()", "sortText": "0001"},
                        {"label": "fake_variable", "kind": 6,
                         "detail": "let fake_variable", "sortText": "0002"},
                    ],
                },
            })
        elif method == "shutdown":
            send({"jsonrpc": "2.0", "id": message["id"], "result": None})
        elif method == "exit":
            return


if __name__ == "__main__":
    main()
