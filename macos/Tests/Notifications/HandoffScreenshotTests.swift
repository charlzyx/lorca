import AppKit
import XCTest
@testable import Lorca

/// Opt-in PR evidence: renders production AppKit controllers with in-process synthetic data.
/// This target is not shipped in the app. No CLI, model, relay, account, or capture permission
/// is used; NSView caches only this fixture's native content, not the user's screen.
@MainActor
final class HandoffScreenshotTests: XCTestCase {
    func testCaptureHandoffStates() async throws {
        guard let folder = ProcessInfo.processInfo.environment["LORCA_HANDOFF_SCREENSHOTS"] else {
            throw XCTSkip("Set LORCA_MOCK=1 and LORCA_HANDOFF_SCREENSHOTS to capture PR evidence")
        }
        guard ProcessInfo.processInfo.environment["LORCA_MOCK"] == "1" else {
            XCTFail("Screenshot fixtures require mock mode")
            return
        }
        let app = NSApplication.shared
        let originalAppearance = app.appearance
        let savedSelection = Preferences.selection
        let savedInspector = Preferences.showsInspector
        Preferences.showsInspector = false
        app.appearance = NSAppearance(named: .aqua)
        let store = AppStore.shared
        XCTAssertTrue(store.isMock)
        store.start()
        for chat in store.chats { store.deleteChat(chat.id) }
        let chef = makeBot("Chef", symbol: "sparkles", accent: .indigo)
        let scout = makeBot("Scout", symbol: "magnifyingglass", accent: .teal)
        let source = store.dm(with: chef)
        let target = store.dm(with: scout)
        let time = Date().addingTimeInterval(-300)
        let handoff = "handoff-00000000-0000-4000-8000-000000000074"
        let responseID = "msg-issue-74-response"
        let reportID = "report-issue-74-completed"

        append(store, source, "msg-request", .you, .text("Ask Scout to review the parser. I need a report and a failing input."), time)
        append(store, source, "msg-delivered", .bot(chef), marker(to: scout), time.addingTimeInterval(10))
        append(store, target, "msg-delegated", .bot(chef), .handoff(from: chef, to: scout,
            reason: "Review the parser. Expected output: a report and a failing input. Acceptance criteria: identify the cause and verify the fix."), time.addingTimeInterval(10))
        append(store, target, responseID, .bot(scout), .text("""
            Parser review complete.

            The empty-field fixture reproduced an unchecked index. The fix checks the field count before indexing.

            - Failing input: `name,,status`
            - Added the regression case to the fixture suite.
            - The three fixture checks pass.
            """), time.addingTimeInterval(40))
        let responseURL = URL(string: "lorca://message?chat_id=\(target)&message_id=\(responseID)")!
        append(store, source, reportID, .bot(scout), .text("""
            Handoff \(handoff) · completed

            Parser review complete. The empty-field fixture now returns a clear error.

            - [Recipient response](\(responseURL.absoluteString))

            Evidence:
            - The failing input reproduced the unchecked index.
            - Three synthetic fixture checks passed.
            """), time.addingTimeInterval(40))
        append(store, source, "msg-continue", .bot(chef), .text("Scout's report is back. I'll review its evidence against the acceptance criteria."), time.addingTimeInterval(45))

        let root = RootSplitViewController()
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1180, height: 760),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.backgroundColor = .windowBackgroundColor
        root.view.frame = window.contentView!.bounds
        window.contentViewController = root
        root.splitViewItems.last?.isCollapsed = true
        window.orderFront(nil)
        defer {
            window.orderOut(nil)
            store.resetMockData()
            Preferences.selection = savedSelection
            Preferences.showsInspector = savedInspector
            app.appearance = originalAppearance
        }
        root.select(.chat(source))
        try await capture(root.view, window: window, folder: folder, name: "01-completed-report.png")

        // Production openLink dispatches the validated internal URL. The fixture installs the
        // same root navigation callback that the application delegate supplies in the app.
        let observer = NotificationCenter.default.addObserver(forName: ChatMessageLink.didOpen, object: nil, queue: .main) { notification in
            guard let link = notification.object as? ChatMessageLink else { return }
            MainActor.assumeIsolated { root.openMessage(chatID: link.chatID, messageID: link.messageID) }
        }
        NSWorkspace.shared.openLink(responseURL)
        NotificationCenter.default.removeObserver(observer)
        XCTAssertEqual(root.selection, .chat(target))
        try await capture(root.view, window: window, folder: folder, name: "02-opened-recipient-response.png")

        // Independent blocked request: retain terminal completion in the first fixture and
        // use a new requesting bot/chat and request id for this case.
        store.deleteChat(source)
        let blockedChef = makeBot("Chef", symbol: "sparkles", accent: .indigo)
        let blockedSource = store.dm(with: blockedChef)
        let blockedMarker = "msg-issue-74-blocked-request"
        let blockedHandoff = "handoff-00000000-0000-4000-8000-000000000075"
        append(store, target, blockedMarker, .bot(blockedChef), .handoff(from: blockedChef, to: scout,
            reason: "Review the schema mapping. Expected output: a verified mapping report. Acceptance criteria: verify against the supplied schema version."), time.addingTimeInterval(60))
        append(store, blockedSource, "msg-blocked-task", .you, .text("Ask Scout to check the schema mapping against the parser report."), time.addingTimeInterval(60))
        append(store, blockedSource, "msg-blocked-delivered", .bot(blockedChef), marker(to: scout), time.addingTimeInterval(65))
        append(store, blockedSource, "report-issue-74-blocked", .bot(scout), .text("""
            Handoff \(blockedHandoff) · blocked

            I need the schema version before I can verify the mapping.

            - [Delegated request](lorca://message?chat_id=\(target)&message_id=\(blockedMarker))

            Evidence:
            - The supplied context contains no schema version.
            - Verification remains incomplete; no completion is claimed.
            """), time.addingTimeInterval(70))
        append(store, blockedSource, "msg-blocked-continue", .bot(blockedChef), .text("Scout is blocked on the schema version. Which version should we use?"), time.addingTimeInterval(75))
        root.select(.chat(blockedSource))
        try await capture(root.view, window: window, folder: folder, name: "03-blocker-report.png")
    }

    private func makeBot(_ name: String, symbol: String, accent: Accent) -> Bot.ID {
        AppStore.shared.createBot(name: name, description: "Synthetic issue #74 screenshot fixture",
            symbolName: symbol, accent: accent, runnerID: "dev-workbench", provider: .deepseek)
    }

    private func marker(to bot: Bot.ID) -> Message.Body {
        .tool(ToolInvocation(name: "message_bot", summary: "Messaged Scout", detail: "Delegated with expected output and acceptance criteria",
            isRunning: false, description: nil, targetBotID: bot, scriptCommand: nil))
    }

    private func append(_ store: AppStore, _ chat: Chat.ID, _ id: Message.ID, _ author: Message.Author, _ body: Message.Body, _ date: Date) {
        XCTAssertEqual(store.append(Message(id: id, author: author, body: body, createdAt: date), to: chat), id)
    }

    private func capture(_ view: NSView, window: NSWindow, folder: String, name: String) async throws {
        // Let native constraints, table row construction, deferred scrolling and text layout settle.
        view.layoutSubtreeIfNeeded()
        try await Task.sleep(nanoseconds: 250_000_000)
        view.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let image = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: image)
        let png = try XCTUnwrap(image.representation(using: .png, properties: [:]))
        let directory = URL(fileURLWithPath: folder, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try png.write(to: directory.appendingPathComponent(name))
        XCTAssertGreaterThan(png.count, 10_000, "Expected rendered AppKit content")
    }
}
