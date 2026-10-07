import AppKit
import XCTest
@testable import Lorca

@MainActor
final class RoutineReliabilityTests: XCTestCase {
    func testRoutineDetailsScrollWhileActionsStayVisible() throws {
        // This UI check runs with LORCA_MOCK=1, so it never starts a real local CLI.
        let store = AppStore.shared
        guard store.isMock else { throw XCTSkip("Run with LORCA_MOCK=1 for the AppKit layout check") }
        store.start()
        let routine = try XCTUnwrap(store.routines.first)
        let bot = try XCTUnwrap(store.bots.first { $0.id == routine.botID })
        let controller = RoutineViewController(routineID: routine.id, bot: bot)
        let view = controller.view
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: view.fittingSize), styleMask: .titled, backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.contentViewController = controller
        defer { window.orderOut(nil) }
        view.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let details = try XCTUnwrap(controller.contentStack.arrangedSubviews.first as? NSScrollView)
        XCTAssertTrue(details.hasVerticalScroller)
        XCTAssertLessThanOrEqual(details.frame.height, 480)
        XCTAssertEqual(controller.contentStack.arrangedSubviews.count, 2, "actions remain outside the details scroll")
        let actions = controller.contentStack.arrangedSubviews[1]
        XCTAssertGreaterThan(actions.frame.height, 20)
        XCTAssertTrue(view.bounds.contains(actions.convert(actions.bounds, to: view)))
        XCTAssertLessThan(view.fittingSize.height, 800)
    }
    func testRoutineHealthAndTimezoneDecodeWithoutUsingDeviceTimezone() throws {
        let data = Data(#"{"id":"rt-1","bot_id":"b1","name":"Brief","prompt":"Read the inbox","schedule":"0 9 * * *","schedule_text":"Every day at 9:00 AM","is_enabled":true,"created_at":0,"timezone":"America/New_York","missed_run_policy":"skip","next_run_at":1791464400,"next_run_text":"2026-10-08 09:00 -04:00 (America/New_York)","state":"quiet","runner_available":true,"health":{"last_check_at":100,"last_success_at":100},"check":"return null"}"#.utf8)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let routine = try decoder.decode(Wire.Routine.self, from: data).toModel()
        XCTAssertEqual(routine.timezone, "America/New_York")
        XCTAssertEqual(routine.missedRunPolicy, "skip")
        XCTAssertEqual(routine.nextSummary, "2026-10-08 09:00 -04:00 (America/New_York)")
        XCTAssertEqual(routine.lastSuccessfulCheckAt, Date(timeIntervalSince1970: 100))
        XCTAssertNil(routine.lastRunAt, "quiet checks are separate from model runs")
        XCTAssertEqual(routine.state, "quiet")
    }

    func testOfflineAndAuthenticationRecoveryDecode() throws {
        let data = Data(#"{"id":"rt-1","bot_id":"b1","name":"Watch","prompt":"Read the inbox","schedule":"every 1h","is_enabled":false,"created_at":0,"state":"blocked","paused_reason":"authentication","runner_available":false,"recovery_action":"Reconnect and resume","retry_at":900}"#.utf8)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let routine = try decoder.decode(Wire.Routine.self, from: data).toModel()
        XCTAssertEqual(routine.timezone, "UTC")
        XCTAssertEqual(routine.missedRunPolicy, "coalesce")
        XCTAssertFalse(routine.runnerAvailable)
        XCTAssertEqual(routine.recoveryAction, "Reconnect and resume")
        XCTAssertEqual(routine.retryAt, Date(timeIntervalSince1970: 900))
        XCTAssertEqual(routine.state, "blocked")
    }
}
