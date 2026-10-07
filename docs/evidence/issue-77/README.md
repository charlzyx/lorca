# Issue 77 UI evidence

These PNGs capture the actual `PlaybookViewController` and `CapturePlaybookViewController` from this branch in synthetic native XCTest fixture windows. The gray “Lorca #77 fixture” titlebar belongs to the harness; the sheet contents and controls come from the production controllers. Captures use macOS Aqua appearance at 2× resolution through Cua Driver's exact-window capture.

The fixture supplies a public-demo review, a bot named Scout, two synthetic revisions, bundled checklist/script text, and explicit source-message selections. It does not start AppStore, connect to a CLI or provider, load an account, or read real chats or credentials. The script text is displayed, never executed. Each state gets a fresh controller/window to preserve native tab and text rendering.

| File | State |
| --- | --- |
| [draft-review.png](draft-review.png) | Inactive draft with explicit bot scope, instructions, and Save Skill |
| [bundled-reference.png](bundled-reference.png) | Bundled reference filename/text with Add File and Remove File |
| [bundled-script.png](bundled-script.png) | Optional script text and the existing-permissions note |
| [correction-history.png](correction-history.png) | Read-only revision history, correction provenance, and synthetic evidence ids |
| [workflow-capture.png](workflow-capture.png) | Explicit project scope with only the request and completed reply selected |
| [correction-evidence.png](correction-evidence.png) | Standing-instruction proposal with one selected correction and the local two-source validation message |

These are newly added views, so there is no corresponding prior authoring screen to compare. The fixtures verify native rendering and the local evidence-count guard. Live provider capture, actual paired-Device relay sync, full manual save-panel/permission flows, and sibling consolidation remain separate PR validation limits.

To reproduce from the repository root, use a new readiness directory on each run. The capture client uses Pillow only to check that document text has painted; it preserves the original native PNG. A temporary Python environment keeps this dependency outside the app:

```sh
python3 -m venv temp/ui-evidence-venv
temp/ui-evidence-venv/bin/python -m pip install Pillow
```

Start the opt-in native fixture test and run the capture client while it waits for exact-window acknowledgments:

```sh
LORCA_PLAYBOOK_CAPTURE_DIR="$PWD/temp/playbook-captures-fresh" \
LORCA_PLAYBOOK_WINDOW_CAPTURES=1 \
swift test --package-path macos --filter PlaybookScreenshotTests
```

```sh
temp/ui-evidence-venv/bin/python docs/evidence/issue-77/capture.py \
  --ready-directory temp/playbook-captures-fresh \
  --output-directory docs/evidence/issue-77
```

The client uses only the fixture's reported pid/window id, checks its exact window title with `list_windows`, captures that window with `get_window_state`, waits for document pixels to paint, and acknowledges it. XCTest's accessibility tree may be unavailable; the capture is verified from the returned window image. No foreground actions are required. The normal test suite skips this opt-in capture test. An individual state can use `LORCA_PLAYBOOK_CAPTURE_ONLY=<state>` in the fixture command and `--only <state>` in the capture client.

The assets and capture client stay under `docs/evidence/`, outside production app resources and bundles. The fixture harness stays in the macOS test target.
