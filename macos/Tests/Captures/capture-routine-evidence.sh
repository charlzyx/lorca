#!/usr/bin/env bash
set -euo pipefail

capture_script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
capture_repo_root="$(git -C "$capture_script_dir" rev-parse --show-toplevel)"
capture_output_dir="$capture_repo_root/docs/review/issue-78"
capture_baseline_ref="02a11cd8d596dfa60d5be28a3156ad7f2c8a6bb4"
capture_generated_source="$capture_repo_root/macos/Tests/Notifications/RoutineBeforeEvidenceGenerated.swift"

if [[ -e "$capture_generated_source" ]]; then
  echo "Refusing to overwrite existing capture source: $capture_generated_source" >&2
  exit 1
fi
trap 'rm -f "$capture_generated_source"' EXIT

# The baseline controller is generated only in the test target and removed afterwards.
# The production target, desktop/, user preferences, and account directories are untouched.
git -C "$capture_repo_root" show "$capture_baseline_ref:macos/Sources/Lorca/Sheets/RoutineViewController.swift" |
  python3 -c 'import sys; print("@testable import Lorca\n" + sys.stdin.read().replace("RoutineViewController", "RoutineBeforeEvidenceGenerated"))' > "$capture_generated_source"

export LORCA_MOCK=1
export LORCA_ROUTINE_EVIDENCE_DIR="$capture_output_dir"
export LORCA_ROUTINE_EVIDENCE_SCENE=before
swift test --package-path "$capture_repo_root/macos" -Xswiftc -DLORCA_CAPTURE_BEFORE --filter RoutineEvidenceCaptureTests

# One process per image avoids AppKit's cached drawing state between fixture windows.
for capture_scene in quiet failed failed-overview blocked blocked-overview waiting_for_runner runner-service; do
  LORCA_ROUTINE_EVIDENCE_SCENE="$capture_scene" swift test --package-path "$capture_repo_root/macos" --skip-build --filter RoutineEvidenceCaptureTests
done
