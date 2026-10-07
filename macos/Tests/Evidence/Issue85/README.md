# Browser Sessions native UI captures

These PNGs capture the real `BrowserSessionsViewController` in an offscreen, borderless AppKit window, using synthetic `browser.sessions` JSON decoded through the same state application and renderer as CLI responses. The fixture uses a demo bot, Runner, account, and stable session id. It starts the in-process mock store and connects to no CLI or browser; it reads no real credentials or private chats.

- `appkit-bot-control.png`: before Take Over; the bot has control.
- `appkit-human-control.png`: after Take Over; Return to Bot is available for the same session.
- `appkit-takeover-waiting.png`: a reported takeover waits for active input; Stop Browser remains enabled.
- `appkit-paired-device.png`: a paired-Device capability response disables visible Open and offers Pause on Runner, with the local interaction limitation shown.

The XCTest capture also checks important button states and that visible buttons fit inside the sheet. These are UI fixture captures; they do not exercise manual button actions, paired networking, or combined sibling workflows. Those QA/consolidation requirements remain in PR #89.

From the assigned checkout:

```sh
LORCA_MOCK=1 LORCA_BROWSER_UI_EVIDENCE_DIR="$PWD/macos/.build/issue85-ui-evidence" swift test --package-path macos --filter 'BrowserSessionCaptureTests|BrowserSessionTests'
```

The test writes fresh PNGs into the requested directory for visual inspection. Evidence assets live under Tests, outside the app executable and bundle resources.
