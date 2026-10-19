#!/usr/bin/env python3
"""A minimal scripted debug adapter for protocol tests.

Speaks just enough of the Debug Adapter Protocol over stdio to exercise
Textchum's client end to end: the handshake, breakpoints, a stop at the
first breakpoint, the stack, scopes and variables (one of them with
children), a step that moves one line down, and an end with an exit
code. A launch whose `program` is "fail" is refused. It never asks the
client anything except, once, for a terminal, which the client must
decline without the session ending.

With `--listen` it speaks over TCP instead, the way Delve does: it
listens on a port of its choosing, says where on its standard output,
and serves one connection.
"""

import json
import socket
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


seq = 0
# Where messages go: standard output, or the connection when listening.
out = sys.stdout.buffer


def send(payload):
    global seq
    seq += 1
    payload["seq"] = seq
    body = json.dumps(payload).encode("utf-8")
    out.write(b"Content-Length: %d\r\n\r\n" % len(body))
    out.write(body)
    out.flush()


def respond(request, body=None, success=True, message=None):
    reply = {"type": "response", "request_seq": request["seq"], "command": request["command"],
             "success": success, "body": body or {}}
    if message:
        reply["message"] = message
    send(reply)


def event(name, body=None):
    send({"type": "event", "event": name, "body": body or {}})


def main():
    global out
    stdin = sys.stdin.buffer
    if "--listen" in sys.argv:
        server = socket.socket()
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        print("DAP server listening at: 127.0.0.1:%d" % server.getsockname()[1], flush=True)
        connection, _ = server.accept()
        stdin = connection.makefile("rb")
        out = connection.makefile("wb")
    program = None
    breakpoints = {}
    line = 1
    while True:
        message = read_message(stdin)
        if message is None:
            return
        if message.get("type") == "response":
            # The client's answer to the terminal it was asked for.
            continue
        command = message.get("command")
        arguments = message.get("arguments") or {}
        if command == "initialize":
            respond(message, {"supportsConfigurationDoneRequest": True})
        elif command == "launch":
            program = arguments.get("program")
            if program == "fail":
                respond(message, success=False, message="no such program: fail")
                continue
            respond(message)
            send({"type": "request", "command": "runInTerminal",
                  "arguments": {"args": ["true"], "cwd": "/"}})
            event("initialized")
        elif command == "setBreakpoints":
            path = arguments["source"]["path"]
            lines = [breakpoint["line"] for breakpoint in arguments.get("breakpoints", [])]
            breakpoints[path] = lines
            respond(message, {"breakpoints": [{"line": n, "verified": True} for n in lines]})
        elif command == "configurationDone":
            respond(message)
            event("output", {"category": "stdout", "output": "hello from the program\n"})
            event("output", {"category": "telemetry", "output": "not for people"})
            first = min((n for lines in breakpoints.values() for n in lines), default=None)
            if first is None:
                event("terminated")
            else:
                line = first
                event("stopped", {"reason": "breakpoint", "threadId": 1})
        elif command == "stackTrace":
            path = next(iter(breakpoints), program)
            respond(message, {"stackFrames": [
                {"id": 1, "name": "main", "source": {"path": path}, "line": line, "column": 1},
                {"id": 2, "name": "start", "source": {"path": path}, "line": 1, "column": 1},
            ]})
        elif command == "scopes":
            respond(message, {"scopes": [
                {"name": "Locals", "variablesReference": 10, "expensive": False},
                {"name": "Registers", "variablesReference": 20, "expensive": True},
            ]})
        elif command == "variables":
            reference = arguments.get("variablesReference")
            if reference == 10:
                respond(message, {"variables": [
                    {"name": "count", "value": "1", "type": "int", "variablesReference": 0},
                    {"name": "shop", "value": "Shop", "type": "Shop", "variablesReference": 11},
                ]})
            else:
                respond(message, {"variables": [
                    {"name": "name", "value": "\"corner\"", "type": "str", "variablesReference": 0},
                ]})
        elif command == "evaluate":
            respond(message, {"result": "2", "variablesReference": 0})
        elif command == "next":
            respond(message)
            line += 1
            event("stopped", {"reason": "step", "threadId": 1})
        elif command == "continue":
            respond(message, {"allThreadsContinued": True})
            event("exited", {"exitCode": 0})
            event("terminated")
        elif command == "disconnect":
            respond(message)
            return
        else:
            respond(message)


if __name__ == "__main__":
    main()
