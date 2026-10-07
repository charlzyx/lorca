import AppKit
import XCTest
@testable import Lorca

#if DEBUG
/// Opt-in PNG evidence from the actual AppKit controllers. The windows stay offscreen;
/// only fixture catalog/progress data is injected, and no local CLI or service is contacted.
@MainActor
final class WorkflowEvidenceTests: XCTestCase {
    private let size = NSSize(width: 800, height: 700)

    func testCaptureWorkflowScreens() throws {
        guard let destination = ProcessInfo.processInfo.environment["LORCA_UI_EVIDENCE_DIR"] else {
            throw XCTSkip("Set LORCA_UI_EVIDENCE_DIR to render the opt-in PR screenshots.")
        }
        guard AppStore.shared.isMock else { throw XCTSkip("UI evidence requires LORCA_MOCK=1.") }
        _ = NSApplication.shared
        AppStore.shared.start()
        let output = URL(fileURLWithPath: destination, isDirectory: true)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let catalogData = try Data(contentsOf: root.appendingPathComponent("crates/cli/marketplace/index.json"))
        let catalog = try Wire.decoder.decode(Wire.Marketplace.self, from: catalogData)
        let packs = try XCTUnwrap(catalog.packs)
        let repository = try XCTUnwrap(packs.first { $0.id == "repository-monitoring" })
        let meeting = try XCTUnwrap(packs.first { $0.id == "meeting-preparation" })
        let fixtureCatalog = Marketplace(packs: packs)

        let onboarding = OnboardingViewController(onFinish: {})
        onboarding.showCompletionCaptureFixture()
        try capture(onboarding, name: "01-onboarding-complete", size: NSSize(width: 660, height: 560), output: output)
        XCTAssertTrue(buttonTitles(onboarding.view).contains("Choose a Workflow…"))

        let outcomes = MarketplaceViewController(captureCatalog: fixtureCatalog, size: size)
        try capture(outcomes, name: "02-outcomes", size: size, output: output)
        XCTAssertEqual(buttonTitles(outcomes.view).filter { $0 == "Set Up…" }.count, packs.count)

        try capturePage(progress(pack: repository, state: "questions"), name: "03-required-questions", catalog: fixtureCatalog, output: output)
        try capturePage(progress(pack: meeting, state: "connections"), name: "04-connection-recovery", catalog: fixtureCatalog, output: output)
        try capturePage(progress(pack: repository, state: "sample"), name: "05-before-sample-review", catalog: fixtureCatalog, output: output)
        try capturePage(progress(pack: repository, state: "reviewed"), name: "06-after-sample-review", catalog: fixtureCatalog, output: output)
        try capturePage(progress(pack: repository, state: "cancelled"), name: "07-cancelled-setup", catalog: fixtureCatalog, output: output)
    }

    private func capturePage(_ progress: WorkflowProgress, name: String, catalog: Marketplace, output: URL) throws {
        let market = MarketplaceViewController(captureCatalog: catalog, size: size)
        _ = market.view
        let page = MarketplaceWorkflowPage(market: market, captureProgress: progress)
        market.show(page, animated: false)
        let titles = buttonTitles(page.view)
        if progress.setup.phase == "sample" {
            XCTAssertTrue(titles.contains("I Have Reviewed This Result"))
            XCTAssertFalse(titles.contains("Enable Schedules"))
        } else if progress.setup.phase == "reviewed" {
            XCTAssertTrue(titles.contains("Enable Schedules"))
            XCTAssertTrue(titles.contains("Finish with Schedules Paused"))
        } else if progress.setup.phase == "cancelled" {
            XCTAssertTrue(titles.contains("Resume Setup"))
        }
        try capture(market, name: name, size: size, output: output)
    }

    private func capture(_ controller: NSViewController, name: String, size: NSSize, output: URL) throws {
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: .borderless, backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.appearance = NSAppearance(named: .aqua)
        window.backgroundColor = .windowBackgroundColor
        window.contentViewController = controller
        let view = try XCTUnwrap(window.contentView)
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        window.layoutIfNeeded()
        view.layoutSubtreeIfNeeded()
        // Allow native view appearance and the onboarding transition to settle offscreen.
        RunLoop.main.run(until: Date(timeIntervalSinceNow: 0.3))
        view.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let representation = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: representation)
        let png = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        try png.write(to: output.appendingPathComponent(name + ".png"), options: .atomic)
        XCTAssertGreaterThan(png.count, 10_000, "Capture must contain rendered controls.")
        window.orderOut(nil)
    }

    private func buttonTitles(_ view: NSView) -> [String] {
        ((view as? NSButton).map { [$0.title] } ?? []) + view.subviews.flatMap(buttonTitles)
    }

    private func progress(pack: WorkflowPack, state: String) throws -> WorkflowProgress {
        let rawPack: [String: Any] = [
            "id": pack.id, "name": pack.name, "outcome": pack.outcome, "description": pack.description,
            "questions": pack.questions.map { ["id": $0.id, "label": $0.label, "placeholder": $0.placeholder] },
            "connections": pack.connections.map { ["service_id": $0.serviceId, "name": $0.name] },
        ]
        let hasSample = state == "sample" || state == "reviewed"
        let sample: Any = hasSample ? [
            "job_id": "job-fixture", "chat_id": "chat-fixture", "bot_id": "bot-fixture",
            "state": state == "reviewed" ? "reviewed" : "ready", "message_ids": ["message-fixture"],
        ] : NSNull()
        let isMeeting = pack.id == "meeting-preparation"
        let role = isMeeting ? "preparer" : "monitor"
        let specialist = isMeeting ? "Meeting Preparer" : "Repository Monitor"
        let selected: [String: String] = isMeeting ? ["google-calendar": "calendar-work-fixture"] : ["github": "github-fixture"]
        let connections: [[String: Any]] = isMeeting ? [
            ["service_id": "google-calendar", "name": "Google Calendar", "selected_id": "calendar-work-fixture", "available": true,
             "state": "needs_auth", "detail": "Sign-in failed. Open the sign-in sheet to try again.",
             "choices": [["id": "calendar-work-fixture", "name": "Google Calendar", "service_id": "google-calendar", "account_name": "Work (fixture)", "state": "needs_auth", "detail": "Sign in"],
                         ["id": "calendar-personal-fixture", "name": "Google Calendar", "service_id": "google-calendar", "account_name": "Personal (fixture)", "state": "ready", "detail": "Connected"]]],
            ["service_id": "google-drive", "name": "Google Drive", "selected_id": NSNull(), "available": false, "state": "missing", "detail": "Google Drive is unavailable in this marketplace. Your setup is saved.", "choices": []],
        ] : [
            ["service_id": "github", "name": "GitHub", "selected_id": "github-fixture", "available": true, "state": "ready", "detail": "Connected · Demo Project (fixture)",
             "choices": [["id": "github-fixture", "name": "GitHub", "state": "ready", "detail": "Connected"]]],
        ]
        let answers: [String: String] = isMeeting ? ["meeting-scope": "Today's external meetings"] : ["repositories": "example/workflow-demo"]
        let body: [String: Any] = [
            "setup": ["id": "workflow-fixture", "runner_id": "dev-workbench", "pack": rawPack,
                      "answers": answers, "bot_ids": state == "questions" ? [:] : [role: "bot-fixture"],
                      "connection_ids": state == "questions" ? [:] : selected, "phase": state, "sample": sample],
            "specialists": [["id": role, "name": specialist, "selected_id": NSNull(), "choices": [["id": "existing-fixture", "name": "Existing Repo Watcher (fixture)"]]]],
            "connections": connections,
            "routines": state == "questions" ? [] : [["id": "routine-fixture", "name": isMeeting ? "Prepare upcoming meetings" : "Monitor selected repositories", "schedule_text": "Weekdays at 9:00 AM", "is_enabled": false]],
            "sample_messages": hasSample ? [["id": "message-fixture", "chat_id": "chat-fixture", "author": ["kind": "bot", "bot_id": "bot-fixture"],
                                             "body": ["kind": "text", "text": "Demo Project: two pull requests need review and one issue needs a reproduction.\nSuggested next step: review the authentication fix, then ask for details on the new issue."],
                                             "state": ["kind": "complete"], "created_at": 1]] : [],
            "is_running": false, "can_sample": !isMeeting && state != "cancelled", "can_enable": state == "reviewed",
            "blocked_reason": isMeeting ? "Finish connecting Google Calendar on Workbench before running a sample." : NSNull(),
        ]
        return try Wire.decoder.decode(WorkflowProgress.self, from: JSONSerialization.data(withJSONObject: body))
    }
}
#endif
