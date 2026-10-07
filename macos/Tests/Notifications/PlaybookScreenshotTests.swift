import AppKit
import XCTest
@testable import Lorca

/// Opt-in evidence captures render the production controllers with synthetic records.
/// No AppStore.start(), CLI connection, model request, or account data is used.
@MainActor
final class PlaybookScreenshotTests: XCTestCase {
    private final class CaptureSurface: NSView {
        override func draw(_ dirtyRect: NSRect) {
            NSColor.windowBackgroundColor.setFill()
            dirtyRect.fill()
        }
    }

    private func descendants<T: NSView>(_ view: NSView, as type: T.Type) -> [T] {
        ((view as? T).map { [$0] } ?? []) + view.subviews.flatMap { descendants($0, as: type) }
    }

    private func capture(_ controller: NSViewController, name: String, directory: URL) throws {
        if let only = ProcessInfo.processInfo.environment["LORCA_PLAYBOOK_CAPTURE_ONLY"], name != only { return }
        let content = controller.view
        content.appearance = NSAppearance(named: .aqua)
        let size = content.fittingSize
        XCTAssertEqual(size.width, 640, accuracy: 1)
        XCTAssertGreaterThan(size.height, 350)
        let surface = CaptureSurface(frame: NSRect(origin: .zero, size: size))
        surface.appearance = content.appearance
        surface.addSubview(content)
        NSLayoutConstraint.activate([
            content.leadingAnchor.constraint(equalTo: surface.leadingAnchor), content.trailingAnchor.constraint(equalTo: surface.trailingAnchor),
            content.topAnchor.constraint(equalTo: surface.topAnchor), content.bottomAnchor.constraint(equalTo: surface.bottomAnchor)
        ])
        let window = NSWindow(contentRect: surface.frame, styleMask: .titled, backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.appearance = surface.appearance
        window.contentView = surface
        window.layoutIfNeeded()
        surface.layoutSubtreeIfNeeded()
        for text in descendants(surface, as: NSTextView.self) {
            if let container = text.textContainer { text.layoutManager?.ensureLayout(for: container) }
        }
        window.displayIfNeeded()
        // This opt-in harness orders its own synthetic fixture behind other windows.
        // A background capture client takes the WindowServer image and acknowledges it.
        window.title = "Lorca #77 fixture · " + name
        window.orderBack(nil)
        let metadata = directory.appendingPathComponent("ready-" + name + ".json")
        let done = directory.appendingPathComponent("captured-" + name)
        let documents = descendants(surface, as: NSScrollView.self).map { scroll -> [String: Any] in
            let text = scroll.documentView as? NSTextView
            let clip = scroll.convert(scroll.contentView.frame, to: surface)
            return ["scroll_frame": NSStringFromRect(scroll.frame), "hidden": scroll.isHidden,
                    "document_frame": scroll.documentView.map { NSStringFromRect($0.frame) } ?? "none",
                    "text_length": text?.string.count ?? 0,
                    "clip_rect": ["x": clip.minX, "y": clip.minY, "width": clip.width, "height": clip.height]]
        }
        let info: [String: Any] = ["pid": ProcessInfo.processInfo.processIdentifier, "window_id": window.windowNumber, "name": name,
                                  "width": window.frame.width, "height": window.frame.height, "documents": documents]
        try JSONSerialization.data(withJSONObject: info, options: .prettyPrinted).write(to: metadata, options: .atomic)
        let deadline = Date().addingTimeInterval(180)
        while !FileManager.default.fileExists(atPath: done.path) && Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        }
        XCTAssertTrue(FileManager.default.fileExists(atPath: done.path), "Fixture window capture was not acknowledged")
        window.orderOut(nil)
        content.removeFromSuperview()
    }

    func testCaptureImplementedPlaybookSheets() throws {
        guard ProcessInfo.processInfo.environment["LORCA_PLAYBOOK_WINDOW_CAPTURES"] == "1",
              let output = ProcessInfo.processInfo.environment["LORCA_PLAYBOOK_CAPTURE_DIR"] else {
            throw XCTSkip("Set LORCA_PLAYBOOK_CAPTURE_DIR to capture synthetic AppKit PR evidence")
        }
        let directory = URL(fileURLWithPath: output, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let content = PlaybookContent(
            name: "weekly-review", description: "Turn a completed weekly review into a repeatable checklist.",
            instructions: """
            1. Read the selected public progress notes.
            2. Compare completed work with the agreed weekly goals.
            3. Summarize changes, open questions, and next steps.
            4. Include a concrete example for every recommendation.
            5. Review the result before sharing it.

            Use references/checklist.md for the review questions.
            Ask through the usual permission flow before any action.
            """,
            examples: "Example: The public demo shipped on Tuesday. The next review checks the remaining accessibility improvements.",
            references: [PlaybookResource(path: "references/checklist.md", text: "# Review questions\n\n- Which public demo goals are complete?\n- What changed since the previous review?\n- Which examples support the recommendations?\n- What needs a decision next week?")],
            scripts: [PlaybookResource(path: "scripts/example.sh", text: "#!/bin/sh\n# Bundled example; saving the skill does not run it.\nprintf '%s\\n' 'Review the public demo checklist.'")]
        )
        let first = PlaybookProvenance(kind: "workflow", chat_id: "fixture-project", message_ids: ["fixture-request", "fixture-reply"],
                                      note: "Drafted from a completed public demo review")
        let corrected = PlaybookProvenance(kind: "corrections", chat_id: "fixture-project", message_ids: ["fixture-correction-1", "fixture-correction-2"],
                                          note: "Repeated corrections ask for a concrete example before recommendations")
        var earlier = content
        earlier.instructions = "Read the public progress notes and summarize the weekly changes."
        let scope = PlaybookScope(kind: "bot", id: "Scout")
        let draft = PlaybookRecord(id: "playbook-fixture", scope: scope, revision: 2, hash: "fixture-hash", status: "draft", content: content,
                                   provenance: corrected, revisions: [
                                    PlaybookRevision(revision: 1, status: "draft", content: earlier, provenance: first, created_at: 1_791_410_400),
                                    PlaybookRevision(revision: 2, status: "draft", content: content, provenance: corrected, created_at: 1_791_414_000)
                                   ])
        for (index, name) in [(0, "draft-review"), (2, "bundled-reference"), (3, "bundled-script"), (4, "correction-history")] {
            let editor = PlaybookViewController(scope: scope, scopeName: "Bot · Scout", record: draft)
            let tabs = try XCTUnwrap(descendants(editor.view, as: NSTabView.self).first)
            tabs.selectTabViewItem(at: index)
            try capture(editor, name: name, directory: directory)
        }

        let start = Date(timeIntervalSince1970: 1_791_410_400)
        let request = Message(id: "fixture-request", author: .you, body: .text("Review this week's public demo progress and suggest next steps."), createdAt: start)
        let reply = Message(id: "fixture-reply", author: .bot("Scout"), body: .text("Completed the review: the demo shipped, and accessibility is the next goal."), createdAt: start.addingTimeInterval(60))
        let one = Message(id: "fixture-correction-1", author: .you, body: .text("Include a concrete example before recommending a next step."), createdAt: start.addingTimeInterval(120))
        let two = Message(id: "fixture-correction-2", author: .you, body: .text("Again, ground the recommendation in a concrete public example."), createdAt: start.addingTimeInterval(180))
        let chat = Chat(id: "fixture-project", kind: .group, customTitle: "Weekly Review", botIDs: ["Scout"], messages: [request, reply, one, two],
                        unreadCount: 0, isPinned: false, createdAt: start)
        let workflow = CapturePlaybookViewController(chat: chat, message: reply)
        try XCTUnwrap(descendants(workflow.view, as: NSPopUpButton.self).first).selectItem(at: 1)
        try capture(workflow, name: "workflow-capture", directory: directory)
        let correction = CapturePlaybookViewController(chat: chat, message: two)
        correction.loadViewIfNeeded()
        correction.confirmTapped() // One selected correction produces local validation, with no inference.
        try capture(correction, name: "correction-evidence", directory: directory)
    }
}
