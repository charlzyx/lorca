import AppKit
import QuartzCore
import XCTest
@testable import Lorca

/// Opt-in evidence capture. The production AppKit controllers render fictional data in
/// offscreen windows; the fixture does not start AppStore's CLI/network lifecycle.
@MainActor
final class ReviewScreenshotTests: XCTestCase {
    func testCaptureReviewScreens() throws {
        guard let path = ProcessInfo.processInfo.environment["LORCA_REVIEW_SCREENSHOT_DIR"] else {
            throw XCTSkip("Set LORCA_REVIEW_SCREENSHOT_DIR and LORCA_MOCK=1 to capture UI evidence.")
        }
        guard ProcessInfo.processInfo.environment["LORCA_MOCK"] == "1" else {
            XCTFail("Screenshot capture requires fictional mock data.")
            return
        }
        _ = NSApplication.shared
        let output = URL(fileURLWithPath: path, isDirectory: true)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let store = AppStore.shared
        store.resetMockData()
        let bot = try XCTUnwrap(store.bots.first)
        let chat = try XCTUnwrap(store.chats.first { $0.isDM && $0.botIDs.contains(bot.id) })

        let draft = try fixture(bot: bot, chat: chat)
        let draftSheet = ReviewViewController(item: draft)
        let draftWindow = host(draftSheet)
        defer { draftWindow.close() }
        try capture(draftSheet.view, to: output.appendingPathComponent("01-editable-draft.png"))

        // Invoke the real local guard with an unsaved edit. It returns before any request.
        let editor = try XCTUnwrap(descendants(draftSheet.view).compactMap { $0 as? NSTextView }.first)
        editor.string += "\n\nCorrection: include the rollout checklist in the Friday update."
        let approve = try button("Approve", in: draftSheet.view)
        approve.performClick(nil)
        XCTAssertTrue(descendants(draftSheet.view).compactMap { $0 as? NSTextField }.contains {
            $0.stringValue == L("Save your changes, then review and approve the new version.")
        })
        try capture(draftSheet.view, to: output.appendingPathComponent("02-unsaved-edit-guard.png"))

        let proposal = try fixture(bot: bot, chat: chat, plugin: true, version: 2)
        let proposalSheet = ReviewViewController(item: proposal)
        let proposalWindow = host(proposalSheet)
        defer { proposalWindow.close() }
        try capture(proposalSheet.view, to: output.appendingPathComponent("03-proposed-call.png"))

        let uncertain = try fixture(bot: bot, chat: chat, plugin: true, version: 2, state: "uncertain")
        let uncertainSheet = ReviewViewController(item: uncertain)
        let uncertainWindow = host(uncertainSheet)
        defer { uncertainWindow.close() }
        for title in ["Save Changes", "Approve", "Reject", "Cancel Item"] {
            XCTAssertFalse(try button(title, in: uncertainSheet.view).isEnabled)
        }
        XCTAssertTrue(try button("Reload", in: uncertainSheet.view).isEnabled)
        try capture(uncertainSheet.view, to: output.appendingPathComponent("04-uncertain-outcome.png"))

        // Render the actual inspector's empty queue through its normal mock-store path.
        // Capture only its production section, excluding unrelated demo conversations.
        let inspector = InspectorViewController()
        _ = inspector.view
        inspector.viewWillAppear()
        inspector.show(selection: .chat(chat.id))
        let inspectorWindow = host(inspector, fixedSize: NSSize(width: 360, height: 900))
        defer { inspectorWindow.close(); inspector.viewDidDisappear() }
        let section = try XCTUnwrap(descendants(inspector.view).compactMap { $0 as? SectionView }.first { $0.title == L("Review queue") })
        XCTAssertTrue(descendants(section).compactMap { $0 as? NSTextField }.contains {
            $0.stringValue == L("Drafts and proposed actions wait here for your review.")
        })
        try capture(section, to: output.appendingPathComponent("00-inspector-queue.png"))
    }

    private func fixture(bot: Bot, chat: Chat, plugin: Bool = false, version: Int = 1, state: String = "pending") throws -> ReviewItem {
        let payload: [String: Any] = plugin ? [
            "kind": "plugin", "plugin_id": "mail-sandbox", "server_name": "main", "tool": "send_message",
            "arguments": ["to": "team@example.test", "subject": "Weekly sandbox update", "body": "The smoke checks passed. Please review the Friday rollout checklist."],
        ] : [
            "kind": "draft", "text": "Hi team,\n\nThe smoke checks passed, and the weekly report is ready for review.\nPlease review the rollout checklist before Friday.\n\nThanks,\n\(bot.name)",
        ]
        var json: [String: Any] = [
            "id": "review-00000000-0000-8000-8000-000000000073", "runner_id": bot.runnerID, "bot_id": bot.id,
            "origin": ["chat_id": chat.id, "message_id": "fixture-routine-marker", "routine_id": "routine-fixture-weekly"],
            "target": ["account": "Editorial sandbox", "resource": plugin ? "Weekly team update" : "Weekly update draft"],
            "rationale": "Review the message before it reaches the team.", "payload": payload,
            "version": version, "revision": version * 2, "state": state,
            "preconditions": ["workdir": "/Users/fixture/Lorca/workspace", "files": plugin ? [["path": "/Users/fixture/Lorca/workspace/weekly-update.md", "hash": "fixture-content-hash"]] : []],
        ]
        if state == "uncertain" {
            json["outcome"] = [
                "summary": "Runner restarted during execution. The action may have completed; inspect the target before creating another proposal.",
                "message_id": "review-status-fixture-73",
            ]
        }
        return try Wire.decoder.decode(ReviewItem.self, from: JSONSerialization.data(withJSONObject: json))
    }

    private func descendants(_ view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap(descendants)
    }

    private func button(_ title: String, in view: NSView) throws -> NSButton {
        try XCTUnwrap(descendants(view).compactMap { $0 as? NSButton }.first { $0.title == L(title) })
    }

    private func host(_ controller: NSViewController, fixedSize: NSSize? = nil) -> NSWindow {
        let view = controller.view
        view.appearance = NSAppearance(named: .aqua)
        view.layoutSubtreeIfNeeded()
        let size = fixedSize ?? view.fittingSize
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: .borderless, backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.backgroundColor = .windowBackgroundColor
        window.contentViewController = controller
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        view.layoutSubtreeIfNeeded()
        CATransaction.flush()
        return window
    }

    private func capture(_ view: NSView, to url: URL) throws {
        // A cropped section also needs the window backdrop behind its translucent header.
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        view.layoutSubtreeIfNeeded()
        let size = view.bounds.size
        XCTAssertGreaterThan(size.width, 100)
        XCTAssertGreaterThan(size.height, 30)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(size.width * 2), pixelsHigh: Int(size.height * 2),
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
        ))
        bitmap.size = size
        view.cacheDisplay(in: view.bounds, to: bitmap)
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url)
        print("AppKit fixture capture: \(url.lastPathComponent) (\(bitmap.pixelsWide)×\(bitmap.pixelsHigh))")
    }
}
