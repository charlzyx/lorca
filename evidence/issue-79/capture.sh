#!/usr/bin/env bash
set -euo pipefail
capture_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$capture_root"
capture_output="${1:-$capture_root/evidence/issue-79}"
mkdir -p "$capture_output" "$capture_root/target"
capture_runtime="$(mktemp -d "$capture_root/target/ui-capture-79.XXXXXX")"
capture_server_pid=""
trap 'if [[ -n "$capture_server_pid" ]]; then kill "$capture_server_pid" 2>/dev/null || true; fi; rm -rf "$capture_runtime"' EXIT
bun evidence/issue-79/fixture-server.ts --port-file "$capture_runtime/port" > "$capture_runtime/server.log" 2>&1 &
capture_server_pid="$!"
for attempt in {1..50}; do
  [[ -s "$capture_runtime/port" ]] && break
  sleep 0.1
done
[[ -s "$capture_runtime/port" ]] || { cat "$capture_runtime/server.log"; exit 1; }
if [[ ! -d macos/Libraries/LorcaMarkdownFFI.xcframework ]]; then
  bun -e 'import { buildMarkdown } from "./scripts/app.ts"; const r = await buildMarkdown("debug"); process.exit(r.ok ? 0 : 1)'
fi
LORCA_PORT="$(cat "$capture_runtime/port")" LORCA_CAPTURE_DIR="$capture_output" LORCA_MOCK=0 \
  swift test --package-path macos --filter BudgetScreenshotCaptureTests
