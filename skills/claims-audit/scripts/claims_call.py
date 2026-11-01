#!/usr/bin/env python3
"""Call claims-mcp tools against a ledger chosen per call, without an MCP client.

An MCP client starts claims-mcp once, with the ledger path fixed when the session starts.
An agent working in another checkout (a git worktree, a subagent's branch) would write the
wrong ledger and pin the wrong commit through it. This script starts its own claims-mcp on
stdio for the ledger you name, makes the calls in order, and stops at the first error.

Usage:
    claims_call.py --catalog PATH TOOL 'JSON-ARGUMENTS'
    claims_call.py --catalog PATH < calls.json   # [{"tool": "...", "arguments": {...}}, ...]

Prints a JSON array with one {"tool", "ok", "result"} per call made. Exits 1 if a call
failed, 2 on a usage or protocol error. Set CLAIMS_MCP to use a binary other than
`claims-mcp` on PATH. Standard library only.
"""

import argparse
import json
import os
import subprocess
import sys

PROTOCOL_VERSION = "2025-06-18"


def read_response(proc, request_id):
    """Read lines from the server until the response to request_id arrives."""
    for line in proc.stdout:
        line = line.strip()
        if not line:
            continue
        message = json.loads(line)
        if message.get("id") == request_id:
            return message
    raise RuntimeError(f"claims-mcp exited before answering request {request_id}")


def send(proc, message):
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()


def decode(text):
    """Tool results are JSON text; return them parsed when they are."""
    try:
        return json.loads(text)
    except ValueError:
        return text


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--catalog", required=True, help="ledger file, e.g. <worktree>/claims.json")
    parser.add_argument("tool", nargs="?", help="tool name; omit to read a JSON list from stdin")
    parser.add_argument("arguments", nargs="?", default="{}", help="tool arguments as JSON")
    args = parser.parse_args()

    # parser.error prints the usage and exits 2, so bad input is never mistaken for a failed call.
    try:
        if args.tool:
            calls = [{"tool": args.tool, "arguments": json.loads(args.arguments)}]
        else:
            calls = json.load(sys.stdin)
    except ValueError as error:
        parser.error(f"invalid JSON: {error}")

    # An absolute path, so the server's cwd cannot change which ledger it opens. The server
    # pins anchors to HEAD of the git repository that holds the ledger, not of the cwd.
    catalog = os.path.abspath(args.catalog)
    binary = os.environ.get("CLAIMS_MCP", "claims-mcp")
    try:
        proc = subprocess.Popen(
            [binary, "--catalog", catalog],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
        )
    except OSError as error:
        print(f"claims_call.py: cannot start {binary}: {error}", file=sys.stderr)
        return 2
    results = []
    status = 0
    try:
        send(proc, {
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "claims_call.py", "version": "1"},
            },
        })
        init = read_response(proc, 0)
        if "error" in init:
            raise RuntimeError(f"initialize failed: {init['error']}")
        send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized"})

        for request_id, call in enumerate(calls, start=1):
            send(proc, {
                "jsonrpc": "2.0", "id": request_id, "method": "tools/call",
                "params": {"name": call["tool"], "arguments": call.get("arguments", {})},
            })
            response = read_response(proc, request_id)
            if "error" in response:
                results.append({"tool": call["tool"], "ok": False, "result": response["error"]})
                status = 1
                break
            result = response["result"]
            texts = [decode(c["text"]) for c in result.get("content", []) if c.get("type") == "text"]
            ok = not result.get("isError", False)
            results.append({
                "tool": call["tool"],
                "ok": ok,
                "result": texts[0] if len(texts) == 1 else texts,
            })
            if not ok:
                status = 1
                break
    except (RuntimeError, ValueError, KeyError) as error:
        print(f"claims_call.py: {error}", file=sys.stderr)
        status = 2
    finally:
        proc.stdin.close()
        proc.wait(timeout=30)

    json.dump(results, sys.stdout, indent=2)
    sys.stdout.write("\n")
    return status


if __name__ == "__main__":
    sys.exit(main())
