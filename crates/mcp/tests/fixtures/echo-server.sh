#!/bin/sh
# A tiny stdio MCP server for tests: answers initialize, tools/list, tools/call
# and ping, one JSON-RPC message per line. Usage: echo-server.sh <out-dir> [mode]
# It writes the names of the variables it got (never their values), its process
# id, its arguments and the methods it was asked for to <out-dir>. Modes: crash,
# silent, orphan.
out=$1
mode=${2:-normal}
env | sed 's/=.*//' | sort > "$out/env-names.$$"
echo "$$" > "$out/pid.$$"
printf '%s\n' "$@" > "$out/args.$$"
case "$mode" in
  crash)
    echo "fatal: the token ${TEST_TOKEN:-} was rejected" >&2
    exit 3 ;;
  silent)
    while IFS= read -r line; do :; done
    exit 0 ;;
  orphan)
    sleep 300 &
    echo "$!" > "$out/child.$$" ;;
esac
while IFS= read -r line; do
  printf '%s\n' "$line" | sed -n 's/.*"method": *"\([^"]*\)".*/\1/p' >> "$out/methods.$$"
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id": *\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*|*'"method": "initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"x8ai-echo","version":"1.0.0"}}}\n' "$id" ;;
    *'"method":"tools/list"'*|*'"method": "tools/list"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"Replies with what it is given","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}]}}\n' "$id" ;;
    *'"method":"tools/call"'*|*'"method": "tools/call"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"echo from x8ai test server"}]}}\n' "$id" ;;
    *)
      [ -n "$id" ] && printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
  esac
done
