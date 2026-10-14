import AppKit
import XCTest
@testable import Lorca

@MainActor
final class OutputTests: XCTestCase {
    private func output(_ id: String, version: Int, minutesAgo: Double, evidence: Output.Evidence? = nil) -> Message {
        Message(
            id: "msg-\(id)-\(version)", author: .bot("bot-1"), body: .text(""), createdAt: Date().addingTimeInterval(-minutesAgo * 60),
            output: Output(id: id, name: "\(id).txt", mime: "text/plain", botId: "bot-1", version: version, evidence: evidence))
    }

    func testAnOutputMessageDecodesWithItsVersionAndCheck() throws {
        let raw = #"""
        {"id":"msg-output-v2","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},
         "body":{"kind":"text","text":"After screenshot · Passed: Checked layout","attachments":[{"id":"att-result2","name":"Screenshot.png","mime":"image/png","size":1024,"width":640,"height":480}]},
         "state":{"kind":"complete"},"created_at":1728000000,
         "output":{"id":"out-result","name":"Screenshot.png","mime":"image/png","bot_id":"bot-1","chat_id":"chat-1","task_id":"task-123","version":2,"previous_message_id":"msg-output-v1",
           "evidence":{"kind":"after_screenshot","summary":"Checked layout","status":"passed","command":"swift test","exit_code":0}}}
        """#
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(raw.utf8)).toModel()
        let output = try XCTUnwrap(message.output)
        XCTAssertEqual(output.id, "out-result")
        XCTAssertEqual(output.version, 2)
        XCTAssertEqual(output.botId, "bot-1")
        XCTAssertEqual(output.symbolName, "photo")
        XCTAssertEqual(output.evidence?.title, "After screenshot")
        XCTAssertEqual(output.evidence?.statusText, "Passed")
        XCTAssertEqual(output.evidence?.command, "swift test")
        XCTAssertEqual(message.attachments.first?.width, 640)
    }

    func testMessagesWithoutAnOutputDecodeAsBefore() throws {
        let raw = #"{"id":"msg-old","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"body":{"kind":"text","text":"hello"},"state":{"kind":"complete"},"created_at":1728000000}"#
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(raw.utf8)).toModel()
        XCTAssertNil(message.output)
        XCTAssertEqual(message.text, "hello")
    }

    func testVersionsGroupUnderTheirOutputNewestFirst() {
        let failed = Output.Evidence(kind: "test_result", summary: "2 fail", status: "failed")
        let series = OutputSeries.group([
            output("out-report", version: 1, minutesAgo: 30, evidence: failed),
            output("out-chart", version: 1, minutesAgo: 20),
            output("out-report", version: 2, minutesAgo: 5),
            Message(author: .bot("bot-1"), body: .text("not an output")),
        ])
        XCTAssertEqual(series.map(\.id), ["out-report", "out-chart"], "the latest published first")
        XCTAssertEqual(series[0].versions.map { $0.output?.version }, [2, 1])
        XCTAssertNil(series[0].output.evidence, "the row shows the latest version")
        XCTAssertTrue(series[0].versions[1].output?.evidence?.failed == true)
    }

    func testAFileThatCouldNotBeFetchedRetriesOnAClick() {
        let tile = AttachmentTile()
        let attachment = Attachment(id: "att-gone", name: "After.png", mime: "image/png", size: 1200, width: 80, height: 60)
        var retries = 0
        tile.configure(attachment, url: nil, onUserBubble: false, error: "the relay no longer has After.png", onRetry: { retries += 1 })
        XCTAssertEqual(tile.toolTip, "After.png: the relay no longer has After.png")
        XCTAssertTrue(tile.accessibilityPerformPress())
        XCTAssertEqual(retries, 1)
        tile.configure(attachment, url: nil, onUserBubble: false, onRetry: { retries += 1 })
        XCTAssertTrue(tile.accessibilityPerformPress())
        XCTAssertEqual(retries, 1, "a file on its way does not retry")
    }

    func testOnlyHTTPSDocumentLinksOpen() {
        var output = Output(id: "out-link", name: "Report", mime: "text/html", botId: "bot-1", version: 1, url: "https://docs.example.com/report")
        XCTAssertEqual(output.documentURL?.host, "docs.example.com")
        XCTAssertEqual(output.symbolName, "link")
        for raw in ["file:///etc/passwd", "javascript:alert(1)", "https://user:secret@example.com", "http://example.com"] {
            output.url = raw
            XCTAssertNil(output.documentURL, raw)
        }
    }

    func testAnOutputSaysWhatKindOfFileItIs() {
        func output(_ name: String, _ mime: String) -> Output {
            Output(id: "out", name: name, mime: mime, botId: "bot-1", version: 1)
        }
        XCTAssertEqual(output("mygo-site/index.html", "text/html").kind, .code)
        XCTAssertEqual(output("mygo-site/index.html", "text/html").typeName, "HTML")
        XCTAssertEqual(output("preview-full.png", "image/png").kind, .image)
        XCTAssertEqual(output("results.csv", "text/csv").kind, .table)
        XCTAssertEqual(output("report.json", "application/json").kind, .data)
        XCTAssertEqual(output("build.tar.gz", "application/gzip").kind, .archive)
        XCTAssertEqual(output("notes", "text/plain").kind, .text)
        XCTAssertEqual(output("notes", "text/plain").typeName, "PLAIN")
        XCTAssertEqual(output("blob", "application/octet-stream").typeName, "")
        let link = Output(id: "out", name: "Report", mime: "text/html", botId: "bot-1", version: 1, url: "https://www.example.com/r")
        XCTAssertEqual(link.kind, .link)
        XCTAssertEqual(link.typeName, "example.com")
    }

    /// An output stands in the transcript as a card without a bubble or a time: the image's
    /// preview, the file's name with its type and size, and the check.
    func testAnOutputIsACardOfItsOwn() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("output-card-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        let shot = folder.appendingPathComponent("preview-full.png")
        let picture = NSImage(size: NSSize(width: 320, height: 1200), flipped: true) { rect in
            NSColor(calibratedRed: 0.24, green: 0.13, blue: 0.42, alpha: 1).setFill()
            rect.fill()
            NSColor.systemPink.setFill()
            NSRect(x: 100, y: 80, width: 120, height: 36).fill()
            return true
        }
        let rep = try XCTUnwrap(NSBitmapImageRep(data: try XCTUnwrap(picture.tiffRepresentation)))
        try XCTUnwrap(rep.representation(using: .png, properties: [:])).write(to: shot)

        let html = Message(
            id: "msg-html", author: .bot("bot-1"), body: .text(""),
            attachments: [Attachment(id: "att-html", name: "mygo-site/index.html", mime: "text/html", size: 18432)],
            output: Output(id: "out-html", name: "mygo-site/index.html", mime: "text/html", botId: "bot-1", version: 1))
        let evidence = Output.Evidence(
            kind: "verification", summary: "Chromium 1280x900 full-page render: 12/12 reveal elements animated in, no console errors.",
            status: "passed", command: "bun run check")
        let image = Message(
            id: "msg-shot", author: .bot("bot-1"), body: .text("Check · Passed: Chromium 1280x900 full-page render"),
            attachments: [Attachment(id: "att-shot", name: "preview-full.png", mime: "image/png", size: 733_000, width: 1280, height: 4800)],
            output: Output(id: "out-shot", name: "preview-full.png", mime: "image/png", botId: "bot-1", version: 1, evidence: evidence))
        XCTAssertTrue(html.isOutputCard)
        XCTAssertFalse(Message(author: .you, body: .text(""), output: html.output).isOutputCard, "the user's own message keeps its bubble")

        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 480, height: 520), styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 480, height: 520))
        root.wantsLayer = true
        root.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        window.contentView = root
        var y: CGFloat = 0
        var cells: [OutputCellView] = []
        for (message, url) in [(html, nil), (image, shot)] as [(Message, URL?)] {
            let cell = OutputCellView()
            let height = OutputCellView.height(for: message, rowWidth: 480, indent: ChatMetrics.horizontalInset) + ChatMetrics.groupTopPadding
            cell.frame = NSRect(x: 0, y: root.bounds.height - y - height, width: 480, height: height)
            cell.configure(output: message.output!, attachment: message.attachments.first, url: url, error: nil, avatar: nil, groupStart: true)
            root.addSubview(cell)
            cells.append(cell)
            y += height
        }
        root.layoutSubtreeIfNeeded()

        let texts = cells.map { cell in Set(descendants(cell).compactMap { ($0 as? NSTextField).flatMap { $0.isHidden ? nil : $0.stringValue } }.filter { !$0.isEmpty }) }
        XCTAssertEqual(texts[0], ["mygo-site/index.html", "HTML · 18 KB"], "no time, no check")
        XCTAssertTrue(texts[1].contains("PNG · 716 KB"))
        XCTAssertTrue(texts[1].contains("Check · Passed"))
        XCTAssertTrue(texts[1].contains("bun run check"))
        XCTAssertFalse(texts[1].contains { $0.contains(Format.time(image.createdAt)) }, "a card shows no time")
        XCTAssertEqual(OutputCellView.height(for: html, rowWidth: 480, indent: ChatMetrics.horizontalInset), 54)
        XCTAssertGreaterThan(OutputCellView.height(for: image, rowWidth: 480, indent: ChatMetrics.horizontalInset), 200 + 54, "the preview stops at 200")

        guard let capture = ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] else { return }
        try FileManager.default.createDirectory(atPath: capture, withIntermediateDirectories: true)
        for (suffix, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            window.appearance = NSAppearance(named: appearance)
            NSAppearance(named: appearance)!.performAsCurrentDrawingAppearance {
                root.layer?.backgroundColor = Theme.transcriptBackground.cgColor
            }
            root.needsDisplay = true
            root.displayIfNeeded()
            let bitmap = try XCTUnwrap(root.bitmapImageRepForCachingDisplay(in: root.bounds))
            root.cacheDisplay(in: root.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: capture).appendingPathComponent("output-cards-\(suffix).png"))
        }
    }

    private func descendants(_ view: NSView) -> [NSView] {
        view.subviews + view.subviews.flatMap(descendants)
    }
}
