import AppKit
import XCTest
@testable import Lorca

/// Opt-in evidence captures from the production AppKit controllers. A read-only local
/// websocket supplies fixture responses; this harness never starts the real CLI/app store.
@MainActor
final class BudgetScreenshotCaptureTests: XCTestCase {
    private let taskID = "task-00000000-0000-4000-8000-000000000079"
    private let interruptedID = "task-00000000-0000-4000-8000-000000000080"
    private let bot = Bot(id: "fixture-bot", name: "Demo Assistant", description: "Screenshot fixture",
                          symbolName: "sparkles", accent: .indigo, runnerID: "fixture-runner",
                          provider: .deepseek, createdAt: Date(timeIntervalSince1970: 0))

    func testCaptureImplementedBudgetAndConnectorStates() async throws {
        guard let directory = ProcessInfo.processInfo.environment["LORCA_CAPTURE_DIR"] else {
            throw XCTSkip("Opt-in capture: run evidence/issue-79/capture.sh")
        }
        XCTAssertNotNil(ProcessInfo.processInfo.environment["LORCA_PORT"])
        let app = NSApplication.shared
        let previousAppearance = app.appearance
        app.appearance = NSAppearance(named: .aqua)
        defer { app.appearance = previousAppearance }
        let client = AppStore.shared.client
        client.connect()
        defer { client.disconnect() }
        try await wait { client.state == .connected }
        struct Info: Decodable { var fixture: String }
        let info = try await client.request("fixture.info", as: Info.self)
        XCTAssertEqual(info.fixture, "issue79-native-fixture", "Only the synthetic localhost fixture may be captured")

        let ready = BudgetViewController(bot: bot, chatID: "fixture-chat", taskID: taskID)
        try await capture(ready, directory: directory, filename: "task-allowance.png", expected: "Ready")
        let exhausted = BudgetViewController(bot: bot, chatID: "fixture-chat", routineID: "routine-fixture")
        try await capture(exhausted, directory: directory, filename: "routine-budget-exhausted.png", expected: "Budget exhausted")
        let interrupted = BudgetViewController(bot: bot, chatID: "fixture-chat", taskID: interruptedID)
        try await capture(interrupted, directory: directory, filename: "task-interrupted.png", expected: "Interrupted — resume explicitly")

        let runner = Device(id: "fixture-runner", name: "Screenshot Runner", model: "Fixture",
                            os: .macos, osVersion: "Test", isThisDevice: true, status: .online,
                            lastSeen: Date(timeIntervalSince1970: 0), machineKey: "fixture-public-key")
        let connector = ConnectorLimitsViewController(pluginID: "fixture-account", runner: runner)
        try await capture(connector, directory: directory, filename: "connector-account-cooldown.png", expected: "Service cooldown until", prefix: true)
        let popup = try XCTUnwrap(descendants(of: connector.view).compactMap { $0 as? NSPopUpButton }.first)
        popup.selectItem(at: 1)
        popup.sendAction(popup.action, to: popup.target)
        try await capture(connector, directory: directory, filename: "connector-service.png", expected: "3 calls active.", prefix: true)
    }

    private func wait(_ condition: () -> Bool) async throws {
        for _ in 0..<250 {
            if condition() { return }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTFail("Fixture response did not reach the production controller")
        throw NSError(domain: "Capture", code: 1)
    }

    private func descendants(of view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap { descendants(of: $0) }
    }

    private func capture(_ controller: SheetViewController, directory: String, filename: String,
                         expected: String, prefix: Bool = false) async throws {
        let view = controller.view
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 700),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .aqua)
        window.backgroundColor = .windowBackgroundColor
        window.contentViewController = controller
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        try await wait {
            controller.confirmButton.isEnabled && descendants(of: view).contains {
                guard let field = $0 as? NSTextField else { return false }
                return prefix ? field.stringValue.hasPrefix(expected) : field.stringValue == expected
            }
        }
        controller.fitSheetToContent()
        let size = controller.preferredContentSize
        XCTAssertGreaterThan(size.width, 400)
        XCTAssertGreaterThan(size.height, 200)
        window.setContentSize(size)
        view.frame = NSRect(origin: .zero, size: size)
        view.layoutSubtreeIfNeeded()
        view.displayIfNeeded()
        let rep = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
        view.cacheDisplay(in: view.bounds, to: rep)
        let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
        try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent(filename))
        print("Native fixture capture: \(filename) · \(rep.pixelsWide)×\(rep.pixelsHigh)")
        window.contentViewController = nil
        window.close()
    }
}
