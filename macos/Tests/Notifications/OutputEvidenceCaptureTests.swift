import AppKit
import XCTest
@testable import Lorca

/// Opt-in captures of the production AppKit views, using only the process-local mock store.
/// Run with LORCA_MOCK=1 LORCA_CAPTURE_OUTPUTS_UI=1; these screenshots do not exercise the relay
/// or the system's external file handlers. Committed evidence lives outside app resources.
@MainActor
final class OutputEvidenceCaptureTests: XCTestCase {
    func testCaptureOutputsSheetStates() async throws {
        guard ProcessInfo.processInfo.environment["LORCA_CAPTURE_OUTPUTS_UI"] == "1" else {
            throw XCTSkip("Opt-in native UI evidence capture")
        }
        let store = AppStore.shared
        guard store.isMock else { throw XCTSkip("Evidence captures require LORCA_MOCK=1") }
        store.start()
        let bot = try XCTUnwrap(store.bots.first)
        let taskID = "task-3a249410-8034-4be2-bf42-9e68b4fe2c41"
        let emptyChat = store.createChat(kind: .group, with: [bot.id], title: "Synthetic output fixture")
        try await captureSheet(chatID: emptyChat, expectedStatus: "No outputs yet.", filename: "outputs-empty.png")

        let versionsChat = store.createChat(kind: .group, with: [bot.id], title: "Synthetic version fixture")
        for version in 1...2 {
            let output = TaskOutput(
                id: "out-fixture-report", name: "Verification report", mime: "text/html",
                botId: bot.id, chatId: versionsChat, taskId: taskID, version: version,
                previousMessageId: version == 2 ? "msg-fixture-report-1" : nil,
                url: "https://docs.example.com/verification",
                evidence: .init(kind: "test_result", summary: version == 2 ? "Sample checks pass (synthetic fixture)." : "Sample layout check fails (synthetic fixture).",
                                status: version == 2 ? "passed" : "failed", command: "sample-test --layout", exitCode: version == 2 ? 0 : 1))
            store.append(Message(id: "msg-fixture-report-\(version)", author: .bot(bot.id), body: .text("Synthetic report output"),
                                 createdAt: Date(timeIntervalSince1970: 1728000000 + Double(version * 60)), output: output), to: versionsChat)
        }
        try await captureSheet(chatID: versionsChat, expectedStatus: "2 output versions", filename: "outputs-versions.png",
                               expectedWords: ["Verification report · v2", "Verification report · v1", "Open document"])

        let temporary = FileManager.default.temporaryDirectory.appendingPathComponent("lorca-output-fixture-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        let chart = SyntheticChart(frame: NSRect(x: 0, y: 0, width: 320, height: 180))
        let chartURL = temporary.appendingPathComponent("Synthetic chart.png")
        try capture(chart, to: chartURL)
        // This public mock-only path caches the file URL as the composer does. It performs no
        // CLI/network operation in mock mode. The bot's displayed avatar is restored afterward.
        store.setBotAvatar(bot.id, fileURL: chartURL)
        var attachment = try XCTUnwrap(store.bot(bot.id)?.avatar)
        attachment.name = "Synthetic chart.png"
        attachment.size = (try Data(contentsOf: chartURL)).count
        attachment.width = 320
        attachment.height = 180
        store.setBotAvatar(bot.id, fileURL: nil)
        let imageChat = store.createChat(kind: .group, with: [bot.id], title: "Synthetic file fixture")
        let imageOutput = TaskOutput(id: "out-fixture-image", name: attachment.name, mime: "image/png", botId: bot.id,
                                     chatId: imageChat, taskId: taskID, version: 1,
                                     evidence: .init(kind: "verification", summary: "Sample generated chart; review remains unverified.", status: "unverified"))
        store.append(Message(id: "msg-fixture-image", author: .bot(bot.id), body: .text("Synthetic file output"),
                             createdAt: Date(timeIntervalSince1970: 1728000300), attachments: [attachment], output: imageOutput), to: imageChat)
        try await captureSheet(chatID: imageChat, expectedStatus: "1 output version", filename: "outputs-file-ready.png",
                               expectedWords: [attachment.name + " · v1", "Preview", "Open", "Save As…"])
    }

    func testCaptureUnavailableFileRow() async throws {
        guard ProcessInfo.processInfo.environment["LORCA_CAPTURE_OUTPUTS_UI"] == "1" else {
            throw XCTSkip("Opt-in native UI evidence capture")
        }
        let store = AppStore.shared
        guard !store.isMock else { throw XCTSkip("Run this capture with LORCA_MOCK=0 for a disconnected CLI") }
        // No start/connect call: the production CLI client returns its disconnected error.
        // Reconstructing the real row after that result renders the same failure/Retry state.
        let attachment = Attachment(id: "att-fixture-unavailable", name: "After screenshot.png", mime: "image/png", size: 1200, width: 640, height: 480)
        let output = TaskOutput(id: "out-fixture-unavailable", name: attachment.name, mime: "image/png", botId: "Fixture bot", chatId: "chat-fixture-offline",
                                taskId: "task-3a249410-8034-4be2-bf42-9e68b4fe2c41", version: 1,
                                evidence: .init(kind: "after_screenshot", summary: "Sample screenshot evidence; file retrieval has not succeeded.", status: "unverified"))
        let message = Message(id: "msg-fixture-unavailable", author: .bot(output.botId), body: .text("Synthetic unavailable file"),
                              createdAt: Date(timeIntervalSince1970: 1728000300), attachments: [attachment], output: output)
        _ = store.localURL(for: attachment, in: output.chatId, messageID: message.id)
        for _ in 0..<100 {
            if store.attachmentError(for: attachment) != nil { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertNotNil(store.attachmentError(for: attachment))
        let row = OutputRow(message: message, store: store)
        let canvas = NSView(frame: NSRect(x: 0, y: 0, width: 560, height: 440))
        row.translatesAutoresizingMaskIntoConstraints = false
        canvas.addSubview(row)
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: canvas.topAnchor, constant: 20),
            row.leadingAnchor.constraint(equalTo: canvas.leadingAnchor, constant: 20),
            row.trailingAnchor.constraint(equalTo: canvas.trailingAnchor, constant: -20),
        ])
        let window = NSWindow(contentRect: canvas.frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.contentView = canvas
        defer { window.orderOut(nil) }
        canvas.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        XCTAssertTrue(labels(in: row).contains("Retry"))
        XCTAssertTrue(labels(in: row).contains { $0.contains("File unavailable:") })
        XCTAssertLessThan(row.frame.height, canvas.frame.height - 40)
        try capture(canvas, to: evidenceDirectory.appendingPathComponent("outputs-file-unavailable.png"))
    }

    private func captureSheet(chatID: Chat.ID, expectedStatus: String, filename: String, expectedWords: [String] = []) async throws {
        let controller = OutputsViewController(chatID: chatID)
        _ = controller.view
        let window = NSWindow(contentRect: controller.view.frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.contentViewController = controller
        defer { window.orderOut(nil) }
        for _ in 0..<100 {
            if labels(in: controller.view).contains(expectedStatus) { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let words = labels(in: controller.view)
        XCTAssertTrue(words.contains(expectedStatus), "Output load did not settle: \(words)")
        for word in expectedWords { XCTAssertTrue(words.contains(word), "Missing production UI control/label: \(word)") }
        controller.view.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        if let row = descendants(in: controller.view).compactMap({ $0 as? OutputRow }).first,
           let scroll = descendants(in: controller.view).compactMap({ $0 as? NSScrollView }).first {
            let firstRow = controller.view.convert(row.bounds, from: row)
            let viewport = controller.view.convert(scroll.contentView.bounds, from: scroll.contentView)
            XCTAssertEqual(firstRow.maxY, viewport.maxY, accuracy: 4, "Short result lists start at the top of the viewport")
        }
        if filename == "outputs-file-ready.png" {
            let buttons = descendants(in: controller.view).compactMap { $0 as? NSButton }
            for title in ["Preview", "Open", "Save As…"] {
                XCTAssertTrue(try XCTUnwrap(buttons.first { $0.title == title }).isEnabled, "Available file action: \(title)")
            }
        }
        try capture(controller.view, to: evidenceDirectory.appendingPathComponent(filename))
    }

    private func descendants(in view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants(in: $0) }
    }

    private func labels(in view: NSView) -> [String] {
        var words: [String] = []
        if let label = view as? NSTextField { words.append(label.stringValue) }
        if let button = view as? NSButton { words.append(button.title) }
        return words + view.subviews.flatMap(labels)
    }

    private var evidenceDirectory: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Tests/Evidence/issue80", isDirectory: true)
    }

    private func capture(_ view: NSView, to url: URL) throws {
        view.appearance = NSAppearance(named: .aqua)
        // NSWindow supplies the sheet background in the app. A content-only offscreen capture
        // needs that same native fill explicitly, otherwise black labels sit on transparency.
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        view.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: url)
    }

    private final class SyntheticChart: NSView {
        override func draw(_ dirtyRect: NSRect) {
            NSColor.white.setFill()
            bounds.fill()
            let title = "Synthetic QA chart" as NSString
            title.draw(at: NSPoint(x: 18, y: 150), withAttributes: [.font: NSFont.systemFont(ofSize: 15, weight: .semibold), .foregroundColor: NSColor.labelColor])
            for (index, height) in [50, 82, 108].enumerated() {
                NSColor.systemBlue.withAlphaComponent(0.5 + Double(index) * 0.2).setFill()
                NSBezierPath(roundedRect: NSRect(x: 30 + index * 90, y: 26, width: 52, height: height), xRadius: 5, yRadius: 5).fill()
            }
        }
    }
}
