import AppKit
import XCTest
@testable import Lorca

@MainActor
final class PlaybookTests: XCTestCase {
    private func descendants<T: NSView>(_ view: NSView, as type: T.Type) -> [T] {
        ((view as? T).map { [$0] } ?? [])
            + view.subviews.flatMap { descendants($0, as: type) }
    }

    func testBundledReferenceEditsStayWithTheSelectedFile() throws {
        let files = PlaybookResourcesView(kind: "references", resources: [
            PlaybookResource(path: "references/first.md", text: "First source"),
            PlaybookResource(path: "references/second.md", text: "Second source")
        ])
        let picker = try XCTUnwrap(descendants(files, as: NSPopUpButton.self).first)
        let editor = try XCTUnwrap(descendants(files, as: NSTextView.self).first)
        editor.string = "Corrected first source"
        picker.selectItem(at: 1)
        _ = picker.sendAction(picker.action, to: picker.target)
        XCTAssertEqual(editor.string, "Second source")
        editor.string = "Corrected second source"
        let value = files.value
        XCTAssertEqual(value.map(\.text), ["Corrected first source", "Corrected second source"])
        let remove = try XCTUnwrap(descendants(files, as: NSButton.self).first { $0.title == L("Remove File") })
        remove.performClick(nil)
        XCTAssertEqual(files.value.map(\.path), ["references/first.md"])
    }

    func testEditorOffersAllContentSectionsAndRetainedHistory() throws {
        let content = PlaybookContent(name: "weekly-review", description: "Review a weekly report", instructions: "Compare evidence", examples: "A public example")
        let provenance = PlaybookProvenance(kind: "corrections", chat_id: "chat", message_ids: ["one", "two"], note: "Repeated correction")
        let record = PlaybookRecord(id: "playbook-test", scope: PlaybookScope(kind: "bot", id: "bot"), revision: 1, hash: "hash", status: "draft",
                                    content: content, provenance: provenance, revisions: [PlaybookRevision(revision: 1, status: "draft", content: content, provenance: provenance, created_at: 100)])
        let controller = PlaybookViewController(scope: record.scope, scopeName: "Test bot", record: record)
        let tabs = try XCTUnwrap(descendants(controller.view, as: NSTabView.self).first)
        XCTAssertEqual(tabs.tabViewItems.map(\.label), [L("Instructions"), L("Examples"), L("References"), L("Scripts"), L("History")])
        let history = try XCTUnwrap(tabs.tabViewItems.last?.view)
        let text = try XCTUnwrap(descendants(history, as: NSTextView.self).first)
        XCTAssertFalse(text.isEditable)
        XCTAssertTrue(text.string.contains("Repeated correction"))
        XCTAssertTrue(text.string.contains("Compare evidence"))
        XCTAssertEqual(controller.confirmButton.title, L("Save Skill"))
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 640, height: 700), styleMask: .borderless, backing: .buffered, defer: false)
        window.contentViewController = controller
        window.layoutIfNeeded()
        XCTAssertGreaterThan(controller.view.fittingSize.height, 400)
        XCTAssertLessThan(controller.view.fittingSize.height, 800)
        window.orderOut(nil)
    }

    func testCorrectionCaptureShowsExplicitScopesAndRequiresRepeatedEvidence() throws {
        let one = Message(id: "one", author: .you, body: .text("Use public examples"))
        let two = Message(id: "two", author: .you, body: .text("Again, use public examples"))
        let failed = Message(id: "failed", author: .you, body: .text("Failed source"), state: .failed("error"))
        let chat = Chat(id: "project", kind: .group, botIDs: ["chef"], messages: [one, two, failed], unreadCount: 0, isPinned: false, createdAt: Date())
        let controller = CapturePlaybookViewController(chat: chat, message: two)
        let picker = try XCTUnwrap(descendants(controller.view, as: NSPopUpButton.self).first)
        XCTAssertEqual(picker.numberOfItems, 2)
        let checkboxes = descendants(controller.view, as: NSButton.self).filter { $0.title.hasPrefix(L("You") + ": ") }
        XCTAssertEqual(checkboxes.count, 2)
        XCTAssertEqual(checkboxes.filter { $0.state == .on }.count, 1)
        controller.confirmTapped()
        XCTAssertTrue(controller.confirmButton.isEnabled, "One correction must fail before starting inference")
        XCTAssertTrue(descendants(controller.view, as: NSTextField.self).contains { $0.stringValue == L("Select 2–20 related user corrections") })
    }
}
