# Issue #74 AppKit capture evidence

These PNGs show this branch's production `RootSplitViewController`, `ChatViewController`, sidebar, message cells, Markdown links, and composer. The opt-in native XCTest harness supplies synthetic Chef/Scout bots and chat messages with the same completion/blocker report text shape as the CLI. It renders them in an offscreen AppKit window and writes the native content view with `NSView.cacheDisplay`; window-manager chrome is outside the capture.

No account, CLI, model, relay, real credential, private chat, external app, or screen-recording permission is used. The Inspector is hidden. The fixture restores its selection, Inspector preference, and AppKit appearance after capture. The harness belongs to the test target, and these evidence assets are outside production bundles.

- `01-completed-report.png`: requesting Chef chat shows the handoff id, completed result, recipient-response link, evidence, and synthetic coordinator continuation.
- `02-opened-recipient-response.png`: after `NSWorkspace.openLink` validates and dispatches the internal URL, the fixture's application-delegate-equivalent callback calls production `RootSplitViewController.openMessage`. The native chat selection changes to Scout and reveals its response. This is the after-navigation view of the same current branch, not a historical before/after implementation comparison.
- `03-blocker-report.png`: independent synthetic blocked request returns the missing-context reason, request link, supporting evidence, and a coordinator question. It does not claim completion.

Reproduce from the assigned worktree:

```sh
LORCA_MOCK=1 LORCA_HANDOFF_SCREENSHOTS="$PWD/docs/pr-evidence/issue-74" \
  swift test --package-path macos --filter HandoffScreenshotTests
```

If the Markdown XCFramework is absent, generate it with the existing `buildMarkdown("debug")` helper in `scripts/app.ts` before running Swift tests.

The fixture test verifies the internal link selects the recipient chat. It does not exercise a physical pointer click, the full running AppDelegate, CLI/relay-driven older-page fetching, live provider completion, or multi-Runner execution. Those existing PR limitations remain. The “three fixture checks” in the rendered report are seeded sample text, not a claim that the screenshot harness ran parser tests.
