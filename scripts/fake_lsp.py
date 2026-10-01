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


def expect_answers(stdin, checks, held):
    """Reads until every request id in `checks` has its answer, holding
    back whatever else arrives meanwhile for the main loop. An answer
    of the wrong shape ends the server, so the client's tests see it
    die rather than carry on against a server they have just broken."""
    while checks:
        message = read_message(stdin)
        if message is None:
            sys.exit(3)
        if message.get("method") is None and message.get("id") in checks:
            check = checks.pop(message["id"])
            if not check(message.get("result")):
                sys.stderr.write("fake_lsp: bad answer %r\n" % (message,))
                sys.stderr.flush()
                sys.exit(3)
        else:
            held.append(message)


def arguments():
    """`--tag NAME` names this server in what it says, so two of them
    on one document can be told apart; `--without A,B` leaves those
    providers out of what it declares at initialize."""
    tag, without = "fake", set()
    rest = sys.argv[1:]
    while rest:
        flag = rest.pop(0)
        if flag == "--tag" and rest:
            tag = rest.pop(0)
        elif flag == "--without" and rest:
            without = set(rest.pop(0).split(","))
    return tag, without


def main():
    tag, without = arguments()
    stdin = sys.stdin.buffer
    # The text of each open document as the client last sent it, so a
    # request can be checked against what the server actually has.
    texts = {}
    seen = 0
    held = []
    # What the client told this server about how to run, for the
    # `fake.report` command to hand back to a test.
    told = {"initializationOptions": None, "workspace": None,
            "textDocument": None, "configuration": None, "changed": None,
            "saved": []}
    while True:
        message = held.pop(0) if held else read_message(stdin)
        if message is None:
            return
        method = message.get("method")
        if method == "initialized":
            # Ask what real servers ask, in the protocol's shape: one
            # entry per configuration item, and the folders as objects.
            send({"jsonrpc": "2.0", "id": 1000, "method": "workspace/configuration",
                  "params": {"items": [{"section": "fake"},
                                       {"section": "fake", "scopeUri": "file:///x"}]}})
            send({"jsonrpc": "2.0", "id": 1001, "method": "workspace/workspaceFolders"})
            def configuration(result):
                told["configuration"] = result
                return isinstance(result, list) and len(result) == 2
            expect_answers(stdin, {
                1000: configuration,
                1001: lambda r: isinstance(r, list) and len(r) == 1
                                and "uri" in r[0] and "name" in r[0],
            }, held)
        elif method == "textDocument/didSave":
            told["saved"].append(message["params"]["textDocument"]["uri"])
        elif method == "workspace/didChangeConfiguration":
            told["changed"] = message["params"].get("settings")
        elif method == "workspace/executeCommand":
            # Only the report is answered; other commands stay as silent
            # as they were.
            if message["params"].get("command") == "fake.report":
                send({"jsonrpc": "2.0", "id": message["id"], "result": told})
        elif method == "initialize":
            told["initializationOptions"] = message["params"].get("initializationOptions")
            told["workspace"] = message["params"].get("capabilities", {}).get("workspace")
            told["textDocument"] = message["params"].get("capabilities", {}).get("textDocument")
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                # Everything this script answers, said out loud: a
                # client that routes by provider asks nobody who has
                # not claimed the request.
                "result": {"capabilities": {
                    name: value for name, value in {
                        "textDocumentSync": 1,
                        "codeActionProvider": {"resolveProvider": True},
                        "hoverProvider": True,
                        "definitionProvider": True,
                        "completionProvider": {},
                        "referencesProvider": True,
                        "renameProvider": True,
                        "documentFormattingProvider": True,
                        "documentSymbolProvider": True,
                        "executeCommandProvider": {"commands": ["fake.report"]},
                    }.items() if name not in without
                }},
            })
        elif method in ("textDocument/didOpen", "textDocument/didChange"):
            seen += 1
            uri = message["params"]["textDocument"]["uri"]
            if method == "textDocument/didOpen":
                texts[uri] = message["params"]["textDocument"].get("text", "")
            elif message["params"].get("contentChanges"):
                texts[uri] = message["params"]["contentChanges"][-1].get("text", "")
            # Crash on demand, for the client's restart tests.
            changes = message["params"].get("contentChanges", [])
            if any(change.get("text") == "die" for change in changes):
                sys.exit(1)
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
                        "message": "%s finding #%d" % (tag, seen),
                        # A server recognizes its own finding by these,
                        # which is why the client has to hand back what
                        # was published rather than a reconstruction.
                        "code": "fake-rule",
                        "source": "fake",
                        "data": {"fix": "quote"},
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
                        "value": tag + " hover at %d:%d"
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
            # A position outside the text the client has sent is a
            # request against text the server has not seen; answering
            # it with STALE lets the client's tests catch the race.
            uri = message["params"]["textDocument"]["uri"]
            position = message["params"]["position"]
            lines = texts.get(uri, "").split("\n")
            line, character = position["line"], position["character"]
            if line >= len(lines) or character > len(lines[line]):
                send({"jsonrpc": "2.0", "id": message["id"],
                      "result": [{"label": "STALE"}]})
                continue
            # A postfix-style item: its own range swallows the five
            # characters before the caret, and an import comes with it.
            postfix = {
                "label": "dbg!", "kind": 3, "filterText": "dbg", "sortText": "0000",
                "textEdit": {
                    "range": {"start": {"line": line, "character": max(0, character - 5)},
                              "end": {"line": line, "character": character}},
                    "newText": "dbg!(x)",
                },
                "additionalTextEdits": [{
                    "range": {"start": {"line": 0, "character": 0},
                              "end": {"line": 0, "character": 0}},
                    "newText": "use std::dbg;\n",
                }],
            }
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "isIncomplete": False,
                    "items": [
                        postfix,
                        {"label": "fake_function", "kind": 3,
                         "detail": "fn fake_function()",
                         "insertText": "fake_function()", "sortText": "0001"},
                        {"label": "fake_variable", "kind": 6,
                         "detail": "let fake_variable", "sortText": "0002"},
                        # A snippet, so the shells' tabstop path has a
                        # real server response to accept.
                        {"label": "fake_snippet", "kind": 3,
                         "detail": "fn fake_snippet(a, b)",
                         "insertText": "fake_snippet(${1:a}, ${2:b})$0",
                         "insertTextFormat": 2, "sortText": "0003"},
                    ],
                },
            })
        elif method == "textDocument/documentSymbol":
            def sym(name, kind, line, children=None):
                node = {
                    "name": name, "kind": kind,
                    "range": {"start": {"line": line, "character": 0},
                              "end": {"line": line, "character": 10}},
                    "selectionRange": {"start": {"line": line, "character": 3},
                                       "end": {"line": line, "character": 8}},
                }
                if children:
                    node["children"] = children
                return node
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": [
                    sym("FakeStruct", 23, 0, [sym("fake_method", 6, 1)]),
                    sym("fake_function", 12, 4),
                ],
            })
        elif method == "textDocument/references":
            uri = message["params"]["textDocument"]["uri"]
            # One of them in a sibling test file, so the shells' split
            # of code from tests has something to split.
            stem, _, extension = uri.rpartition(".")
            test_uri = f"{stem}_test.{extension}" if stem else uri
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": [
                    {"uri": uri,
                     "range": {"start": {"line": line, "character": 0},
                               "end": {"line": line, "character": 4}}}
                    for line in (0, 2)
                ] + [
                    {"uri": test_uri,
                     "range": {"start": {"line": 1, "character": 0},
                               "end": {"line": 1, "character": 4}}}
                ],
            })
        elif method == "textDocument/codeAction":
            uri = message["params"]["textDocument"]["uri"]
            handed_back = message["params"].get("context", {}).get("diagnostics", [])
            actions = []
            # Only offer the fix when the client handed back the
            # diagnostic as published, `data` and all.
            if any(d.get("data", {}).get("fix") == "quote" for d in handed_back):
                actions.append({
                    "title": "Quote the first word",
                    "kind": "quickfix",
                    "isPreferred": True,
                    "edit": {"changes": {uri: [{
                        "range": {
                            "start": {"line": 0, "character": 0},
                            "end": {"line": 0, "character": 0},
                        },
                        "newText": '"',
                    }]}},
                })
            # And one with no edit, for the resolve path.
            actions.append({"title": "Think about it", "kind": "refactor"})
            send({"jsonrpc": "2.0", "id": message["id"], "result": actions})
        elif method == "codeAction/resolve":
            resolved = dict(message["params"])
            resolved["edit"] = {"changes": {}}
            send({"jsonrpc": "2.0", "id": message["id"], "result": resolved})
        elif method == "textDocument/rename":
            uri = message["params"]["textDocument"]["uri"]
            new_name = message["params"]["newName"]
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "changes": {
                        uri: [{
                            "range": {"start": {"line": 0, "character": 0},
                                      "end": {"line": 0, "character": 4}},
                            "newText": new_name,
                        }],
                    },
                },
            })
        elif method == "textDocument/formatting":
            send({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": [{
                    "range": {"start": {"line": 0, "character": 0},
                              "end": {"line": 0, "character": 0}},
                    "newText": "formatted: ",
                }],
            })
        elif method == "shutdown":
            send({"jsonrpc": "2.0", "id": message["id"], "result": None})
        elif method == "exit":
            return


if __name__ == "__main__":
    main()
