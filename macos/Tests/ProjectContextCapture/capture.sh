#!/bin/bash
set -euo pipefail
CAPTURE_REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
CAPTURE_OUTPUT_DIR="${1:-$CAPTURE_REPO_ROOT/docs/evidence/issue-83}"
CAPTURE_BUILD_DIR="$(mktemp -d /tmp/lorca-83-capture.XXXXXX)"
trap 'rm -rf "$CAPTURE_BUILD_DIR"' EXIT
# Keep native helper definitions byte-for-byte from the production file. The remainder of
# Controls.swift contains unrelated app controls and is outside this sheet fixture.
python3 - "$CAPTURE_REPO_ROOT" "$CAPTURE_BUILD_DIR" <<'PY'
import pathlib, sys
root, output = map(pathlib.Path, sys.argv[1:])
source = (root / 'macos/Sources/Lorca/Design/Controls.swift').read_text()
start = source.index('enum Build {')
end = source.index('/// A button that briefly confirms', start)
background = source.index('class BackgroundView: NSView {')
brace = source.index('{', background)
level = 1
pos = brace + 1
while level:
    level += (source[pos] == '{') - (source[pos] == '}')
    pos += 1
(output / 'ProductionBuild.swift').write_text('import AppKit\n\n' + source[start:end] + source[background:pos] + '\n')
PY
swiftc -swift-version 5 -module-name ProjectContextCapture \
    "$CAPTURE_BUILD_DIR/ProductionBuild.swift" \
    "$CAPTURE_REPO_ROOT/macos/Sources/Lorca/Model/ProjectContext.swift" \
    "$CAPTURE_REPO_ROOT/macos/Sources/Lorca/Sheets/SheetViewController.swift" \
    "$CAPTURE_REPO_ROOT/macos/Sources/Lorca/Sheets/ProjectContextViewController.swift" \
    "$CAPTURE_REPO_ROOT/macos/Tests/ProjectContextCapture/Capture.swift" \
    -o "$CAPTURE_BUILD_DIR/capture"
TZ=UTC "$CAPTURE_BUILD_DIR/capture" "$CAPTURE_OUTPUT_DIR"
