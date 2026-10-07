import AppKit
import XCTest
@testable import Lorca

@MainActor
final class OutputTests: XCTestCase {
    func testAnOutputVersionRetainsProvenanceAttachmentsAndVerification() throws {
        let raw = #"""
        {"id":"msg-output-v2","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},
         "body":{"kind":"text","text":"Output: Screenshot.png · v2","attachments":[{"id":"att-result2","name":"Screenshot.png","mime":"image/png","size":1024,"width":640,"height":480}]},
         "state":{"kind":"complete"},"created_at":1728000000,
         "output":{"id":"out-result","name":"Screenshot.png","mime":"image/png","bot_id":"bot-1","chat_id":"chat-1","task_id":"task-123","version":2,"previous_message_id":"msg-output-v1",
           "evidence":{"kind":"after_screenshot","summary":"Checked layout","status":"passed","command":"swift test","exit_code":0}}}
        """#
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(raw.utf8)).toModel()
        let output = try XCTUnwrap(message.output)
        XCTAssertEqual(output.id, "out-result")
        XCTAssertEqual(output.version, 2)
        XCTAssertEqual(output.previousMessageId, "msg-output-v1")
        XCTAssertEqual(output.botId, "bot-1")
        XCTAssertEqual(output.chatId, "chat-1")
        XCTAssertEqual(output.taskId, "task-123")
        XCTAssertEqual(output.evidence?.title, "After screenshot")
        XCTAssertEqual(output.evidence?.statusText, "Passed")
        XCTAssertEqual(output.evidence?.exitCode, 0)
        XCTAssertEqual(message.attachments.first?.width, 640)
        let layout = ChatLayout()
        let metrics = layout.metrics(for: message, showsAvatar: false, groupStart: true, tableWidth: 540)
        XCTAssertGreaterThan(metrics.attachmentsSize.height, 100)
        XCTAssertGreaterThan(metrics.bubbleHeight, metrics.attachmentsSize.height)
    }

    func testExistingMessagesDecodeWithoutOutputMetadata() throws {
        let raw = #"{"id":"msg-old","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"body":{"kind":"text","text":"hello"},"state":{"kind":"complete"},"created_at":1728000000}"#
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(raw.utf8)).toModel()
        XCTAssertNil(message.output)
        XCTAssertEqual(message.text, "hello")
    }

    func testUnavailableFilesShowRetryInsteadOfAnEndlessFetch() {
        let tile = AttachmentTile()
        let attachment = Attachment(id: "att-gone", name: "After.png", mime: "image/png", size: 1200, width: 80, height: 60)
        var retries = 0
        tile.configure(attachment, url: nil, onUserBubble: false, error: "the relay no longer has After.png", onRetry: { retries += 1 })
        XCTAssertTrue(tile.availabilityText.contains("unavailable"))
        XCTAssertTrue(tile.availabilityText.contains("Retry"))
        XCTAssertTrue(tile.toolTip?.contains("no longer has") == true)
        XCTAssertTrue(tile.accessibilityPerformPress())
        XCTAssertEqual(retries, 1)
        tile.configure(attachment, url: nil, onUserBubble: false)
        XCTAssertEqual(tile.availabilityText, "Fetching…")
    }

    func testOutputDetailsAndUnavailablePreviewFitInANativeResultCard() throws {
        let output = TaskOutput(
            id: "out-layout", name: "Verification report for the generated dashboard and its before and after screenshots", mime: "text/html",
            botId: "Chef", chatId: "chat-1", taskId: "task-3a249410-8034-4be2-bf42-9e68b4fe2c41", version: 2,
            url: "https://docs.example.com/verification", evidence: .init(kind: "test_result", summary: "All focused checks pass. The output retains its task, producing bot and version so another paired Device can inspect this evidence.", status: "passed", command: "cargo test -p lorca outputs::tests --lib", exitCode: 0))
        let message = Message(author: .bot("Chef"), body: .text("Output"), output: output)
        let row = OutputRow(message: message, store: .shared)
        row.translatesAutoresizingMaskIntoConstraints = false
        let tile = AttachmentTile()
        tile.translatesAutoresizingMaskIntoConstraints = false
        tile.configure(Attachment(id: "att-after", name: "After.png", mime: "image/png", size: 1200, width: 640, height: 480), url: nil, onUserBubble: false, error: "The relay no longer has After.png", onRetry: {})
        let content = NSStackView(views: [row, tile])
        content.orientation = .vertical
        content.alignment = .leading
        content.spacing = 16
        content.translatesAutoresizingMaskIntoConstraints = false
        let canvas = NSView(frame: NSRect(x: 0, y: 0, width: 460, height: 360))
        canvas.wantsLayer = true
        canvas.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        canvas.addSubview(content)
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: canvas.topAnchor, constant: 20),
            content.leadingAnchor.constraint(equalTo: canvas.leadingAnchor, constant: 20),
            content.trailingAnchor.constraint(equalTo: canvas.trailingAnchor, constant: -20),
            row.widthAnchor.constraint(equalTo: content.widthAnchor),
            tile.widthAnchor.constraint(equalTo: content.widthAnchor),
            tile.heightAnchor.constraint(equalToConstant: 52),
        ])
        let window = NSWindow(contentRect: canvas.frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.contentView = canvas
        canvas.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        defer { window.orderOut(nil) }
        XCTAssertGreaterThan(row.frame.height, 160)
        XCTAssertLessThan(content.frame.height, canvas.frame.height - 20)
        for label in row.arrangedSubviews.compactMap({ $0 as? NSTextField }) {
            XCTAssertLessThanOrEqual(label.alignmentRect(forFrame: label.frame).maxX, row.bounds.width + 1, label.stringValue)
            XCTAssertGreaterThan(label.frame.height, 0)
        }
        let bitmap = try XCTUnwrap(canvas.bitmapImageRepForCachingDisplay(in: canvas.bounds))
        canvas.cacheDisplay(in: canvas.bounds, to: bitmap)
        let directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent(".build/output-evidence", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: directory.appendingPathComponent("native-output-card.png"))
    }

    func testDocumentActionsOnlyOpenHTTPSReferences() {
        var output = TaskOutput(id: "out-link", name: "Report", mime: "text/html", botId: "bot-1", chatId: "chat-1", version: 1, url: "https://docs.example.com/report")
        XCTAssertEqual(output.documentURL?.host, "docs.example.com")
        for raw in ["file:///etc/passwd", "javascript:alert(1)", "https://user:secret@example.com", "http://example.com"] {
            output.url = raw
            XCTAssertNil(output.documentURL, raw)
        }
    }
}
