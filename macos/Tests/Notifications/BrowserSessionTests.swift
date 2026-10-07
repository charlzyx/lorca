import XCTest
@testable import Lorca

final class BrowserSessionTests: XCTestCase {
    func testRunnerOwnershipProfileAndControlSurviveTheWire() throws {
        let json = #"{"sessions":[{"id":"browser-123","bot_id":"bot-1","runner_id":"runner-2","account":"Work","profile":"Research","state":"human","selected":true,"revision":9}],"capabilities":{"visible_open":false,"local_input":false,"remote_live_view":false,"remote_input":false,"native_input":false}}"#
        let list = try JSONDecoder().decode(BrowserSessionList.self, from: Data(json.utf8))
        let session = try XCTUnwrap(list.sessions.first)
        XCTAssertEqual(session.botID, "bot-1")
        XCTAssertEqual(session.runnerID, "runner-2")
        XCTAssertEqual(session.title, "Work · Research")
        XCTAssertEqual(session.revision, 9)
        XCTAssertTrue(session.canReturnToBot)
        XCTAssertFalse(session.isStopped)
        XCTAssertFalse(list.capabilities.visibleOpen)
        XCTAssertFalse(list.capabilities.localInput)
        XCTAssertFalse(list.capabilities.remoteInput)
        XCTAssertFalse(list.capabilities.remoteLiveView)
        XCTAssertFalse(list.capabilities.nativeInput)
    }

    func testPendingTakeoverAndStoppedSessionsCannotReturnControl() throws {
        for state in ["taking_over", "stopped"] {
            let json = "{\"id\":\"s\",\"bot_id\":\"b\",\"runner_id\":\"r\",\"account\":\"a\",\"profile\":\"p\",\"state\":\"\(state)\",\"selected\":true,\"revision\":2}"
            let session = try JSONDecoder().decode(BrowserSession.self, from: Data(json.utf8))
            XCTAssertFalse(session.canReturnToBot)
            XCTAssertEqual(session.isTakingOver, state == "taking_over")
            XCTAssertEqual(session.isStopped, state == "stopped")
        }
    }
}
