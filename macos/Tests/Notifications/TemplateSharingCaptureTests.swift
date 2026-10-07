import AppKit
import XCTest
@testable import Lorca

/// Actual production AppKit controllers with synthetic CLI replies. This exercises their
/// native controls and review gating without a CLI, account files, credentials, or chat data.
@MainActor
final class TemplateSharingCaptureTests: XCTestCase {
    private let profile: [String: Any] = ["name": "Release Reviewer", "description": "Review pull requests. Summarize risks and report the checks run.", "symbol_name": "checklist", "accent": "blue"]
    private let memory = "Use concise reviews; contact review@example.test."
    private let routine: [String: Any] = ["name": "Morning pull-request scan", "schedule": "every 2h", "prompt": "Summarize open pull requests and flag missing checks."]

    private var template: [String: Any] {
        ["format": "lorca.bot-template", "version": 1, "profile": profile,
            "memories": [memory], "routines": [routine], "requirements": [["service_id": "github"]]]
    }

    private var warnings: [[String: Any]] {
        [["path": "memories.0", "message": "Selected memory may contain personal or project information. Contains an email address."]]
    }

    private var contents: [String: Any] {
        ["profile": profile, "skills": [], "memories": [["id": "fixture-memory", "content": memory]],
            "routines": [["id": "fixture-routine", "content": routine]], "requirements": [["service_id": "github"]],
            "notes": ["Reusable skills need playbook support in this CLI. Update Lorca before exporting or importing skills."]]
    }

    func testNativeExportSelectionAndReviewedPreview() async throws {
        try prepare()
        let bot = Bot(id: "fixture-source-bot", name: "Release Reviewer", description: "Review pull requests.", symbolName: "checklist", accent: .blue, runnerID: "dev-workbench", provider: .deepseek, createdAt: Date(timeIntervalSince1970: 1))
        var requestedSelection: [String: Any]?
        let controller = TemplateExportViewController(bot: bot) { [unowned self] method, params in
            switch method {
            case "templates.contents": return contents
            case "templates.export.preview":
                requestedSelection = params["selection"] as? [String: Any]
                return ["template": template, "digest": "fixture-reviewed-digest", "warnings": warnings, "visibility": "private_file"]
            default: XCTFail("Fixture must not save files: \(method)"); return [:]
            }
        }
        let window = host(controller)
        defer { window.orderOut(nil) }
        try await wait { self.buttons(in: controller.view).contains { $0.title.hasPrefix("Profile and instructions") } }
        XCTAssertFalse(controller.confirmButton.isEnabled)
        try capture(controller, window: window, name: "export-selection")

        for title in ["Profile and instructions: Release Reviewer", memory, "Morning pull-request scan", "github"] {
            try XCTUnwrap(buttons(in: controller.view).first { $0.title == title }).performClick(nil)
        }
        XCTAssertTrue(controller.confirmButton.isEnabled)
        controller.confirmButton.performClick(nil)
        try await wait { controller.confirmButton.title == "Save Private File…" }
        XCTAssertFalse(controller.confirmButton.isEnabled, "saving requires explicit personal-content review")
        XCTAssertEqual(requestedSelection?["profile"] as? Bool, true)
        XCTAssertEqual(requestedSelection?["memory_ids"] as? [String], ["fixture-memory"])
        XCTAssertEqual(requestedSelection?["routine_ids"] as? [String], ["fixture-routine"])
        XCTAssertEqual(requestedSelection?["requirement_ids"] as? [String], ["github"])
        try XCTUnwrap(buttons(in: controller.view).first { $0.title.hasPrefix("I reviewed the selected") }).performClick(nil)
        XCTAssertTrue(controller.confirmButton.isEnabled)
        try capture(controller, window: window, name: "export-review")
    }

    func testNativeImportConnectionChoiceAndPausedRoutinePreview() async throws {
        try prepare()
        var chosen: String?
        let controller = TemplateImportViewController(url: URL(fileURLWithPath: "/fixture/reviewer.lorca-template"), reply: { [unowned self] method, params in
            XCTAssertEqual(method, "templates.import.preview")
            chosen = (params["mappings"] as? [String: String])?["github"]
            return ["template": template, "digest": "fixture-import-digest", "warnings": warnings,
                "can_import": chosen == "github", "issues": chosen == "github" ? [] : ["Select your own connection for github."],
                "requirements": [["service_id": "github", "candidates": [["id": "github", "name": "GitHub · Demo workspace", "state": "ready", "detail": "Ready"]]]]]
        }, onCreate: { _ in XCTFail("Fixture must not import a bot") })
        let window = host(controller)
        defer { window.orderOut(nil) }
        try await wait { self.popups(in: controller.view).contains { $0.identifier?.rawValue == "github" } }
        XCTAssertFalse(controller.confirmButton.isEnabled)
        try capture(controller, window: window, name: "import-needs-connection")
        let popup = try XCTUnwrap(popups(in: controller.view).first { $0.identifier?.rawValue == "github" })
        popup.selectItem(at: 1)
        _ = popup.sendAction(popup.action, to: popup.target)
        try await wait { self.buttons(in: controller.view).contains { $0.title.hasPrefix("I reviewed the contents") && $0.isEnabled } }
        XCTAssertEqual(chosen, "github")
        XCTAssertFalse(controller.confirmButton.isEnabled, "connection choice still requires explicit review")
        try XCTUnwrap(buttons(in: controller.view).first { $0.title.hasPrefix("I reviewed the contents") }).performClick(nil)
        XCTAssertTrue(controller.confirmButton.isEnabled)
        XCTAssertTrue(descendants(controller.view).compactMap { ($0 as? NSTextView)?.string }.contains { $0.contains("Imported paused") })
        try capture(controller, window: window, name: "import-reviewed")
    }

    func testNativeImportExplainsMissingPlaybookCapability() async throws {
        try prepare()
        let controller = TemplateImportViewController(url: URL(fileURLWithPath: "/fixture/reviewer-with-skill.lorca-template"), reply: { [unowned self] method, _ in
            XCTAssertEqual(method, "templates.import.preview")
            var content = template
            content["skills"] = [["name": "review-checklist", "description": "Review a proposed change", "instructions": "Read the diff, identify risks, and report tests."]]
            return ["template": content, "digest": "fixture-blocked-digest", "can_import": false, "requirements": [], "warnings": warnings,
                "issues": ["Reusable skills need playbook support in this CLI. Update Lorca before exporting or importing skills."]]
        }, onCreate: { _ in XCTFail("Fixture must not import a bot") })
        let window = host(controller)
        defer { window.orderOut(nil) }
        try await wait { self.descendants(controller.view).compactMap { ($0 as? NSTextField)?.stringValue }.contains { $0.hasPrefix("Reusable skills need playbook support") } }
        XCTAssertFalse(controller.confirmButton.isEnabled)
        try capture(controller, window: window, name: "import-missing-capability")
    }

    private func prepare() throws {
        guard ProcessInfo.processInfo.environment["LORCA_MOCK"] == "1" else {
            throw XCTSkip("Native fixtures require LORCA_MOCK=1 and never connect to a real CLI")
        }
        _ = NSApplication.shared
        XCTAssertTrue(AppStore.shared.isMock)
        AppStore.shared.start()
    }

    private func host(_ controller: SheetViewController) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 640, height: 760), styleMask: .borderless, backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .aqua)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.contentViewController = controller
        window.orderFront(nil)
        return window
    }

    private func capture(_ controller: SheetViewController, window: NSWindow, name: String) throws {
        controller.view.layoutSubtreeIfNeeded()
        window.setContentSize(NSSize(width: 640, height: ceil(controller.view.fittingSize.height)))
        controller.view.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bounds = controller.confirmButton.convert(controller.confirmButton.bounds, to: controller.view)
        XCTAssertTrue(controller.view.bounds.contains(bounds), "confirmation remains inside the native sheet")
        guard let folder = ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] else { return }
        let view = try XCTUnwrap(window.contentView?.superview)
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        let output = URL(fileURLWithPath: folder)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        try png.write(to: output.appendingPathComponent(name + ".png"))
    }

    private func wait(_ condition: () -> Bool) async throws {
        for _ in 0..<100 {
            if condition() { return }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTFail("Native fixture did not reach its expected UI state")
        throw CLIClient.RequestError(message: "Fixture timed out")
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    private func buttons(in view: NSView) -> [NSButton] { descendants(view).compactMap { $0 as? NSButton } }
    private func popups(in view: NSView) -> [NSPopUpButton] { descendants(view).compactMap { $0 as? NSPopUpButton } }
}
