import AppKit
import XCTest
@testable import Lorca

/// Opt-in native evidence captures. The production controllers render a synthetic CLI
/// snapshot; no CLI process, account files, provider, relay, or service is contacted.
@MainActor
final class RoutineEvidenceCaptureTests: XCTestCase {
    func testCaptureRoutineReliabilityScreens() async throws {
        guard let directory = ProcessInfo.processInfo.environment["LORCA_ROUTINE_EVIDENCE_DIR"] else {
            throw XCTSkip("Set LORCA_ROUTINE_EVIDENCE_DIR and LORCA_MOCK=1 to capture native UI evidence")
        }
        let store = AppStore.shared
        guard store.isMock else { throw XCTSkip("Evidence capture requires LORCA_MOCK=1") }
        let output = URL(fileURLWithPath: directory, isDirectory: true)
        try FileManager.default.createDirectory(at: output, withIntermediateDirectories: true)
        _ = NSApplication.shared
        let priorAppearance = NSApp.appearance
        NSApp.appearance = NSAppearance(named: .aqua)
        defer { NSApp.appearance = priorAppearance }
        store.start()

        let scene = ProcessInfo.processInfo.environment["LORCA_ROUTINE_EVIDENCE_SCENE"] ?? "quiet"
        #if LORCA_CAPTURE_BEFORE
        if scene == "before" {
            store.apply(snapshot: try fixture("quiet"))
            let bot = try XCTUnwrap(store.bots.first)
            let controller = RoutineBeforeEvidenceGenerated(routineID: "rt-fixture-78", bot: bot)
            let window = makeWindow(controller, size: controller.view.fittingSize)
            defer { window.orderOut(nil) }
            try await capture(window, to: output.appendingPathComponent("routine-before.png"))
            return
        }
        #endif
        if scene != "runner-service" {
            let state = scene.replacingOccurrences(of: "-overview", with: "")
            XCTAssertTrue(["quiet", "failed", "blocked", "waiting_for_runner"].contains(state))
            store.apply(snapshot: try fixture(state))
            let bot = try XCTUnwrap(store.bots.first)
            let controller = RoutineViewController(routineID: "rt-fixture-78", bot: bot)
            let window = makeWindow(controller, size: controller.view.fittingSize)
            defer { window.orderOut(nil) }
            let fields = descendants(controller.view).compactMap { $0 as? NSTextField }.map(\.stringValue)
            XCTAssertTrue(fields.contains(try XCTUnwrap(store.routines.first).stateText))
            let run = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSButton }.first { $0.title == "Run Now" })
            XCTAssertEqual(run.isEnabled, state == "quiet" || state == "failed")
            if (state == "failed" || state == "blocked") && !scene.hasSuffix("-overview") {
                let scroll = try XCTUnwrap(controller.contentStack.arrangedSubviews.first as? NSScrollView)
                // Show recovery/history in the same bounded sheet a user scrolls through.
                scroll.contentView.scroll(to: NSPoint(x: 0, y: 145))
                scroll.reflectScrolledClipView(scroll.contentView)
            }
            let region: NSView? = scene.hasSuffix("-overview") || state == "waiting_for_runner" ? controller.contentStack : nil
            try await capture(window, region: region, to: output.appendingPathComponent("routine-\(scene).png"))
            return
        }

        store.apply(snapshot: try fixture("quiet"))
        let controller = AboutDeviceSettingsViewController()
        _ = controller.view
        controller.show(deviceID: "runner-fixture-78")
        let window = makeWindow(controller, size: NSSize(width: 720, height: 640))
        defer { window.orderOut(nil) }
        let check = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSButton }.first { $0.title == "Check status" })
        check.performClick(nil)
        for _ in 0..<30 {
            if descendants(controller.view).compactMap({ $0 as? NSTextField }).contains(where: { $0.stringValue == "Not installed" }) { break }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertTrue(descendants(controller.view).compactMap { $0 as? NSTextField }.contains { $0.stringValue == "Not installed" })
        try await capture(window, to: output.appendingPathComponent("runner-service.png"))
    }

    private func fixture(_ state: String) throws -> Wire.Snapshot {
        let at = try XCTUnwrap(ISO8601DateFormatter().date(from: "2026-10-08T13:00:00Z")).timeIntervalSince1970
        let enabled = state != "blocked"
        let online = state != "waiting_for_runner"
        var routine: [String: Any] = [
            "id": "rt-fixture-78", "bot_id": "bot-fixture-78", "name": "Inbox monitor",
            "prompt": "Summarize new messages in the sample inbox. Report only items that need a decision.",
            "check": "// Synthetic inbox fixture\nreturn null;", "schedule": "0 9 * * 1-5",
            "schedule_text": "Weekdays at 9:00 AM", "timezone": "America/New_York",
            "missed_run_policy": online ? "coalesce" : "skip", "is_enabled": enabled,
            "state": state, "runner_available": online, "is_running": false, "created_at": at - 86_400,
            "health": ["last_check_at": at, "last_success_at": at - (state == "quiet" ? 0 : 3_600)],
        ]
        if enabled {
            routine["next_run_at"] = at + 86_400
            routine["next_run_text"] = "2026-10-09 09:00 -04:00 (America/New_York)"
        }
        if state == "failed" {
            routine["retry_at"] = at + 3_600
            routine["recovery_action"] = "Check the connection on the assigned Runner. The routine retries automatically after its backoff."
        } else if state == "blocked" {
            routine["paused_reason"] = "authentication"
            routine["recovery_action"] = "Reconnect the provider in Settings or sign in to the integration on the assigned Runner, then resume this routine."
        } else if !online {
            routine["recovery_action"] = "Start Lorca or lorca serve on the assigned Runner. For an owned computer that stays available, install the CLI service with lorca service install."
        }
        let data: [String: Any] = [
            "version": "0.1.10", "has_identity": true, "is_identity_device": false,
            "identity_id": "synthetic-account-78", "this_device_id": "fixture-viewer-78",
            "relay_url": "https://relay.example.test", "relay_connected": true,
            "devices": [["id": "runner-fixture-78", "name": "Sample Mac mini", "model": "Mac mini",
                "os": "macos", "os_version": "macOS 26", "machine_key": "synthetic-runner-key-78",
                "is_this_device": false, "status": online ? "online" : "offline", "last_seen": at - 60]],
            "bots": [["id": "bot-fixture-78", "name": "Scout", "description": "Watches the sample inbox.",
                "symbol_name": "tray", "accent": "blue", "runner_id": "runner-fixture-78", "provider": "deepseek", "created_at": at - 86_400]],
            "chats": [], "providers": [], "models": [], "running_chat_ids": [], "routines": [routine],
        ]
        return try Wire.decoder.decode(Wire.Snapshot.self, from: JSONSerialization.data(withJSONObject: data))
    }

    private func makeWindow(_ controller: NSViewController, size: NSSize) -> NSWindow {
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: .borderless, backing: .buffered, defer: false)
        let left = NSScreen.screens.map { $0.frame.minX }.min() ?? 0
        let bottom = NSScreen.screens.map { $0.frame.minY }.min() ?? 0
        window.setFrameOrigin(NSPoint(x: left - size.width - 100, y: bottom - size.height - 100))
        window.appearance = NSAppearance(named: .aqua)
        let background = BackgroundView()
        background.fillColor = .windowBackgroundColor
        background.cornerRadius = 0
        window.contentView = background
        background.addSubview(controller.view)
        controller.view.pin(to: background)
        background.layoutSubtreeIfNeeded()
        window.orderFront(nil)
        window.displayIfNeeded()
        return window
    }

    private func capture(_ window: NSWindow, region: NSView? = nil, to url: URL) async throws {
        try await Task.sleep(nanoseconds: 100_000_000)
        let view = try XCTUnwrap(window.contentView)
        view.layoutSubtreeIfNeeded()
        for child in [view] + descendants(view) { child.needsDisplay = true }
        window.display()
        view.display()
        let rect = region.map { $0.convert($0.bounds, to: view) } ?? view.bounds
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: rect))
        view.cacheDisplay(in: rect, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: url)
        XCTAssertGreaterThan(png.count, 10_000)
    }

    private func descendants(_ view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants($0) }
    }
}
