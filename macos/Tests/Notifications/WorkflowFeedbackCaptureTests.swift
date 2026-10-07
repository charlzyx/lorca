import AppKit
import XCTest
@testable import Lorca

/// Native, offscreen captures of the production sheets. Synthetic responses replace only
/// the request boundary; there is no CLI connection, account, relay or real conversation.
@MainActor
final class WorkflowFeedbackCaptureTests: XCTestCase {
    private let bot = Bot(
        id: "fixture-chef", name: "Chef", description: "Synthetic screenshot fixture",
        symbolName: "sparkles", accent: .indigo, runnerID: "fixture-runner", provider: .deepseek, createdAt: Date(timeIntervalSince1970: 0))
    private let chatID = "fixture-morning-brief"
    private var windows: [NSWindow] = []

    private final class Responses {
        var state: [String: Any]
        var calls: [(String, [String: Any])] = []
        init(_ state: [String: Any]) { self.state = state }
        func request(_ botID: Bot.ID, _ method: String, _ params: [String: Any]) async throws -> [String: Any] {
            precondition(botID == "fixture-chef")
            calls.append((method, params))
            switch method {
            case "feedback.list": return state
            case "feedback.accept":
                precondition(params["diff_hash"] as? String == "fixture-diff-hash")
                state["proposals"] = []
                state["revisions"] = [[
                    "id": "fixture-revision-1", "version": 1, "state": "applied",
                    "can_rollback": true, "current_hash": "fixture-current-hash",
                    "rollback_diff": "--- current\n+++ previous\n@@ -1,2 +1,1 @@\n-Summarize the inbox.\n-Put the summary first, then list action items.\n+Summarize the inbox.\n",
                ]]
                return [:]
            case "feedback.exclude":
                state["settings"] = ["review_every_secs": 604800, "excluded_chats": ["fixture-morning-brief"]]
                state["proposals"] = []
                state["revisions"] = []
                state["feedback"] = [
                    ["id": "fixture-edit", "kind": "edited", "excluded": true, "origin": Self.origin],
                    ["id": "fixture-ignored", "kind": "ignored_alert", "excluded": false,
                     "note": "Another included chat: no response was recorded; no preference is inferred.", "example": "Routine alert: nothing needs a decision today.",
                     "origin": ["chat_id": "fixture-neutral-alerts", "message_id": "fixture-alert-1"]],
                ]
                return [:]
            case "feedback.record": return [:]
            default: throw NSError(domain: "CaptureFixture", code: 1, userInfo: [NSLocalizedDescriptionKey: "Unexpected request \(method)"])
            }
        }
        static var origin: [String: Any] {
            ["chat_id": "fixture-morning-brief", "message_id": "fixture-draft-1", "routine_id": "Morning brief"]
        }
        static func pending() -> Responses {
            Responses([
                "settings": ["review_every_secs": 604800, "excluded_chats": []],
                "targets": [["name": "Morning brief", "target": ["kind": "routine_prompt", "id": "Morning brief"]]],
                "proposals": [[
                    "id": "fixture-proposal-1", "state": "pending", "diff_hash": "fixture-diff-hash",
                    "target": ["kind": "routine_prompt", "id": "Morning brief"],
                    "explanation": "You edited two briefs to put the summary first. Keep that order in future briefs.",
                    "origins": [origin],
                    "diff": "--- current\n+++ proposed\n@@ -1,1 +1,2 @@\n-Summarize the inbox.\n+Summarize the inbox.\n+Put the summary first, then list action items.\n",
                ]],
                "feedback": [[
                    "id": "fixture-edit", "kind": "edited", "excluded": false,
                    "note": "Put the summary first, followed by the action items.",
                    "example": "Three messages need a reply today.", "origin": origin,
                ]],
                "revisions": [],
            ])
        }
    }

    override func tearDown() async throws {
        windows.forEach { $0.orderOut(nil) }
        windows.removeAll()
    }

    private func allViews(_ view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap(allViews)
    }

    private func button(_ title: String, in view: NSView) throws -> NSButton {
        try XCTUnwrap(allViews(view).compactMap { $0 as? NSButton }.first { $0.title == title })
    }

    private func waitFor(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Native fixture did not settle")
        throw NSError(domain: "CaptureFixture", code: 2)
    }

    private func mount(_ controller: NSViewController) -> NSView {
        let view = controller.view
        view.appearance = NSAppearance(named: .aqua)
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        let size = view.fittingSize
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: .borderless, backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.setFrameOrigin(NSPoint(x: -5000, y: -5000))
        window.contentView = view
        window.setContentSize(size)
        window.layoutIfNeeded()
        view.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        windows.append(window)
        return view
    }

    private func scrollTo(_ text: String, in view: NSView) throws {
        let label = try XCTUnwrap(allViews(view).compactMap { $0 as? NSTextField }.first { $0.stringValue == text })
        let scroll = try XCTUnwrap(view.subviews.flatMap(allViews).compactMap { $0 as? NSScrollView }.first { $0.documentView is FlippedView })
        let document = try XCTUnwrap(scroll.documentView)
        let rect = label.convert(label.bounds, to: document)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: max(0, rect.minY - 8)))
        scroll.reflectScrolledClipView(scroll.contentView)
        view.layoutSubtreeIfNeeded()
    }

    private func capture(_ view: NSView, _ name: String, directory: URL) throws {
        view.layoutSubtreeIfNeeded()
        let bounds = view.bounds
        XCTAssertGreaterThan(bounds.width, 500)
        XCTAssertGreaterThan(bounds.height, 300)
        let bitmap = try XCTUnwrap(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(bounds.width * 2), pixelsHigh: Int(bounds.height * 2),
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        bitmap.size = bounds.size
        view.cacheDisplay(in: bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: directory.appendingPathComponent(name), options: .atomic)
        print("Native fixture capture: \(name), \(bitmap.pixelsWide)×\(bitmap.pixelsHigh)")
    }

    func testCaptureChangedWorkflowFeedbackSheets() async throws {
        guard let path = ProcessInfo.processInfo.environment["LORCA_UI_EVIDENCE_DIR"] else {
            throw XCTSkip("Set LORCA_UI_EVIDENCE_DIR to render native PR evidence")
        }
        let directory = URL(fileURLWithPath: path, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        _ = NSApplication.shared
        let responses = Responses.pending()
        let request: WorkflowFeedbackRequest = { bot, method, params in try await responses.request(bot, method, params) }

        let message = Message(id: "fixture-draft-1", author: .bot(bot.id), body: .text("Action items: reply to three messages.\nSummary: the inbox is clear apart from those replies."))
        let recorder = RecordWorkflowFeedbackViewController(botID: bot.id, chatID: chatID, message: message, feedbackRequest: request)
        let recordView = mount(recorder)
        try await waitFor { self.allViews(recordView).compactMap { $0 as? NSPopUpButton }.contains { $0.numberOfItems == 2 } }
        let menus = allViews(recordView).compactMap { $0 as? NSPopUpButton }
        try XCTUnwrap(menus.first { $0.itemTitles.contains("User edited") }).selectItem(withTitle: "User edited")
        try XCTUnwrap(menus.first { $0.itemTitles.contains("Morning brief") }).selectItem(withTitle: "Morning brief")
        try XCTUnwrap(allViews(recordView).compactMap { $0 as? NSTextField }.first { $0.placeholderString == "Your feedback or explanation" }).stringValue = "Put the summary first, then list action items."
        try XCTUnwrap(allViews(recordView).compactMap { $0 as? NSTextView }.first).string = "Summary: three messages need a reply today.\nAction items: reply to the three flagged messages."
        try capture(recordView, "01-record-user-edit.png", directory: directory)
        try button("Record", in: recordView).performClick(nil)
        try await waitFor { responses.calls.contains { $0.0 == "feedback.record" } }
        let recorded = try XCTUnwrap(responses.calls.first { $0.0 == "feedback.record" }?.1["feedback"] as? [String: Any])
        XCTAssertEqual(recorded["kind"] as? String, "edited")
        XCTAssertNotEqual(recorded["before"] as? String, recorded["after"] as? String)

        let reviewer = WorkflowFeedbackViewController(bot: bot, chatID: chatID, feedbackRequest: request)
        let reviewView = mount(reviewer)
        try await waitFor { self.allViews(reviewView).compactMap { $0 as? NSButton }.contains { $0.title == "Accept revision" } }
        try scrollTo("Proposed improvements", in: reviewView)
        try capture(reviewView, "02-review-proposed-diff.png", directory: directory)
        try button("Accept revision", in: reviewView).performClick(nil)
        try await waitFor { self.allViews(reviewView).compactMap { $0 as? NSButton }.contains { $0.title == "Roll back this revision" } }
        try scrollTo("Revision history", in: reviewView)
        try capture(reviewView, "03-accepted-rollback-preview.png", directory: directory)
        XCTAssertTrue(responses.calls.contains { $0.0 == "feedback.accept" && $0.1["diff_hash"] as? String == "fixture-diff-hash" })

        try button("Exclude this chat", in: reviewView).performClick(nil)
        try await waitFor { self.allViews(reviewView).compactMap { $0 as? NSButton }.contains { $0.title == "This chat is excluded" } }
        let outer = try XCTUnwrap(allViews(reviewView).compactMap { $0 as? NSScrollView }.first { $0.documentView is FlippedView })
        outer.contentView.scroll(to: .zero)
        outer.reflectScrolledClipView(outer.contentView)
        XCTAssertFalse(try button("This chat is excluded", in: reviewView).isEnabled)
        try capture(reviewView, "04-exclusions-and-neutral-alert.png", directory: directory)
    }
}
