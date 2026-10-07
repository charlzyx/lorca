import AppKit
import XCTest
@testable import Lorca

/// Opt-in captures of the real AppKit sheet with synthetic protocol responses.
/// The harness neither connects to the CLI nor opens a browser or private chat.
@MainActor
final class BrowserSessionCaptureTests: XCTestCase {
    func testCaptureSessionAndTakeoverControls() throws {
        guard let destination = ProcessInfo.processInfo.environment["LORCA_BROWSER_UI_EVIDENCE_DIR"] else {
            throw XCTSkip("Set LORCA_BROWSER_UI_EVIDENCE_DIR and LORCA_MOCK=1 to capture native UI fixtures.")
        }
        _ = NSApplication.shared
        let appearance = try XCTUnwrap(NSAppearance(named: .aqua))
        let previousAppearance = NSApp.appearance
        NSApp.appearance = appearance
        defer { NSApp.appearance = previousAppearance }
        let store = AppStore.shared
        guard store.isMock else { throw XCTSkip("Native evidence captures require LORCA_MOCK=1.") }
        store.start()
        let output = URL(fileURLWithPath: destination, isDirectory: true)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)

        let runner = Device(id: "runner-demo", name: "Demo Runner", model: "Demo Mac", os: .macos, osVersion: "", isThisDevice: true, status: .online, lastSeen: Date(timeIntervalSince1970: 0), machineKey: "demo-only")
        let bot = Bot(id: "bot-demo", name: "Demo Research Bot", description: "Synthetic browser UI fixture", symbolName: "globe", accent: .blue, runnerID: runner.id, provider: .deepseek, createdAt: Date(timeIntervalSince1970: 0))

        for (name, state, local) in [
            ("appkit-bot-control", "bot", true),
            ("appkit-human-control", "human", true),
            ("appkit-takeover-waiting", "taking_over", true),
            ("appkit-paired-device", "bot", false),
        ] {
            let payload: [String: Any] = [
                "sessions": [[
                    "id": "browser-00000000-0000-4000-8000-000000000085",
                    "bot_id": bot.id, "runner_id": runner.id, "account": "Demo work account", "profile": "Research", "state": state, "selected": true, "revision": 3,
                ]],
                "capabilities": ["visible_open": local, "local_input": local, "remote_live_view": false, "remote_input": false, "native_input": false],
            ]
            let response = try JSONDecoder().decode(BrowserSessionList.self, from: JSONSerialization.data(withJSONObject: payload))
            let sheet = BrowserSessionsViewController(bot: bot, chatID: "chat-demo", runner: runner)
            let root = sheet.view
            root.appearance = appearance
            root.wantsLayer = true
            appearance.performAsCurrentDrawingAppearance {
                root.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
            }
            sheet.applySessionList(response)

            let descendants = views(in: root)
            let buttons = descendants.compactMap { $0 as? NSButton }
            XCTAssertTrue(try XCTUnwrap(buttons.first { $0.title == L("Stop Browser") }).isEnabled)
            XCTAssertEqual(try XCTUnwrap(buttons.first { $0.title == L("Open Browser") }).isEnabled, local && state != "taking_over")
            let controlTitle = state == "human" ? L("Return to Bot") : (local ? L("Take Over") : L("Pause on Runner"))
            XCTAssertEqual(try XCTUnwrap(buttons.first { $0.title == controlTitle }).isEnabled, state != "taking_over")
            XCTAssertEqual(try XCTUnwrap(buttons.first { $0.title == L("Attach Screenshot") }).isEnabled, state != "taking_over")

            let size = root.fittingSize
            XCTAssertGreaterThan(size.width, 500)
            XCTAssertGreaterThan(size.height, 300)
            let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: .borderless, backing: .buffered, defer: false)
            window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
            window.appearance = root.appearance
            window.backgroundColor = .windowBackgroundColor
            window.contentView = root
            root.setFrameSize(size)
            root.layoutSubtreeIfNeeded()
            for view in descendants { view.needsDisplay = true }
            window.displayIfNeeded()
            defer { window.orderOut(nil) }

            for button in buttons where !button.isHidden {
                XCTAssertTrue(root.bounds.insetBy(dx: -1, dy: -1).contains(button.convert(button.bounds, to: root)), "\(name): \(button.title) fits inside the sheet")
            }
            let bitmap = try XCTUnwrap(root.bitmapImageRepForCachingDisplay(in: root.bounds))
            root.cacheDisplay(in: root.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            try png.write(to: output.appendingPathComponent(name + ".png"))
        }
    }

    private func views(in root: NSView) -> [NSView] {
        [root] + root.subviews.flatMap { views(in: $0) }
    }
}
