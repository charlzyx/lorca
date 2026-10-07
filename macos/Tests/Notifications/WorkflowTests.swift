import XCTest
@testable import Lorca

final class WorkflowTests: XCTestCase {
    func testOlderMarketplaceRepliesDecodeWithNoPacks() throws {
        let wire = try Wire.decoder.decode(Wire.Marketplace.self, from: Data(#"{"plugins":[],"bots":[]}"#.utf8))
        XCTAssertNil(wire.packs)
    }

    func testNamedSelectionAndReviewedSampleSurviveWireDecoding() throws {
        let source = #"""
        {
          "setup": {
            "id":"workflow-1","runner_id":"runner-1",
            "pack":{"id":"inbox-triage","name":"Inbox triage","outcome":"Triage the inbox","description":"Review a sample","questions":[{"id":"inbox-scope","label":"Which messages?","placeholder":"Unread"}],"connections":[{"service_id":"gmail","name":"Gmail"}]},
            "answers":{"inbox-scope":"Unread today"},"bot_ids":{"triager":"bot-1"},"connection_ids":{"gmail":"gmail-personal"},"phase":"reviewed",
            "sample":{"job_id":"job-1","chat_id":"chat-1","bot_id":"bot-1","state":"reviewed","message_ids":["reply-1"]}
          },
          "connections":[{"service_id":"gmail","name":"Gmail","selected_id":"gmail-personal","choices":[{"id":"gmail-work","name":"Gmail","service_id":"gmail","account_name":"Work","state":"ready","detail":"Connected"},{"id":"gmail-personal","name":"Gmail","service_id":"gmail","account_name":"Personal","state":"ready","detail":"Connected"}],"available":true,"state":"ready","detail":"Connected"}],
          "specialists":[],"routines":[{"id":"routine-1","name":"Inbox check","schedule_text":"Weekdays at 9:00 AM","is_enabled":false}],
          "sample_messages":[{"id":"reply-1","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"body":{"kind":"text","text":"One urgent message."},"state":{"kind":"complete"},"created_at":1}],
          "is_running":false,"can_sample":true,"can_enable":true,"blocked_reason":null
        }
        """#
        let progress = try Wire.decoder.decode(WorkflowProgress.self, from: Data(source.utf8))
        XCTAssertEqual(progress.setup.answers["inbox-scope"], "Unread today")
        XCTAssertEqual(progress.connections[0].selectedId, "gmail-personal")
        XCTAssertEqual(progress.connections[0].choices[0].label, "Gmail · Work")
        XCTAssertEqual(progress.setup.sample?.jobId, "job-1")
        XCTAssertEqual(progress.sampleMessages.first?.body.text, "One urgent message.")
        XCTAssertTrue(progress.canEnable)
        XCTAssertFalse(progress.routines[0].isEnabled)
    }
}
