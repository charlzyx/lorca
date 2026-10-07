import AppKit
import XCTest
@testable import Lorca

final class DurableTaskTests: XCTestCase {
    @MainActor
    func testTaskEditorFitsItsNativeSheetAndKeepsLongFormsScrollable() {
        let sheet = DurableTaskViewController(chatID: "test-chat", task: nil)
        sheet.loadView()
        sheet.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(sheet.view.fittingSize.width, 590, accuracy: 1)
        XCTAssertGreaterThan(sheet.view.fittingSize.height, 510)
        XCTAssertLessThan(sheet.view.fittingSize.height, 800)
        XCTAssertTrue(sheet.contentStack.arrangedSubviews.contains { $0 is NSScrollView })
    }
    func testWireDecodesOwnershipStateAndImmutableEvidenceReferences() throws {
        let data = Data(#"""
        {"id":"task-00000000-0000-0000-0000-000000000001","revision":7,
         "authority_runner_id":"authority","owner_bot_id":"bot","runner_id":"runner",
         "goal":"Deliver report","acceptance_criteria":["Sources verified"],"dependencies":["task-dependency"],
         "next_action":"Review evidence","chat_ids":["group"],"links":[],"state":"awaiting_review",
         "reason":null,"result":"Report ready","active_run":null,"created_at":1,"updated_at":2,
         "evidence":[{"kind":"output","label":"Report v2","chat_id":"group","message_id":"message-v2","output_id":"out-report","version":2}]}
        """#.utf8)
        let task = try Wire.decoder.decode(DurableTask.self, from: data)
        XCTAssertEqual(task.authorityRunnerId, "authority")
        XCTAssertEqual(task.ownerBotId, "bot")
        XCTAssertEqual(task.runnerId, "runner")
        XCTAssertEqual(task.state, .awaitingReview)
        XCTAssertFalse(task.state.canRun)
        XCTAssertEqual(task.evidence[0].params["output_id"] as? String, "out-report")
        XCTAssertEqual(task.evidence[0].params["version"] as? UInt64, 2)
        XCTAssertEqual(task.evidence[0].params["message_id"] as? String, "message-v2")
        XCTAssertEqual(Set(DurableTask.State.allCases.map(\.rawValue)), ["queued", "working", "blocked", "awaiting_review", "completed", "cancelled"])
    }

    func testEvidenceParametersKeepMessageAndChatScope() {
        let evidence = DurableTask.Evidence(kind: "message", label: "Verified", chatId: "chat", messageId: "message")
        XCTAssertEqual(evidence.params["chat_id"] as? String, "chat")
        XCTAssertEqual(evidence.params["message_id"] as? String, "message")
        XCTAssertNil(evidence.params["output_id"])
        XCTAssertNil(evidence.params["url"])
    }
}
