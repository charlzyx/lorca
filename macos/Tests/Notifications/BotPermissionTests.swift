import XCTest
@testable import Lorca

final class BotPermissionTests: XCTestCase {
    func testPoliciesKeepInstanceIDsAndEmptyAllowlistsThroughTheWire() throws {
        let json = """
            {"connections":{"gmail-0123456789abcdef0123456789abcdef":{"capabilities":["read","draft"],"tools":["list_messages","create_draft"]}},"tools":[],"filesystem":"read","shell":false}
            """
        let policy = try Wire.decoder.decode(BotPermissions.self, from: Data(json.utf8))
        XCTAssertEqual(policy.tools, [])
        XCTAssertEqual(policy.connections?.keys.sorted(), ["gmail-0123456789abcdef0123456789abcdef"])
        XCTAssertEqual(policy.connections?.values.first?.capabilities, ["read", "draft"])
        XCTAssertEqual(policy.filesystem, "read")
        XCTAssertFalse(policy.shell)
        let roundtrip = try Wire.decoder.decode(BotPermissions.self, from: JSONSerialization.data(withJSONObject: policy.json))
        XCTAssertEqual(roundtrip, policy)
        let legacy = try Wire.decoder.decode(BotPermissions.self, from: Data("{}".utf8))
        XCTAssertNil(legacy.connections)
        XCTAssertNil(legacy.tools)
        XCTAssertTrue(legacy.shell)
        XCTAssertEqual(legacy.filesystem, "write")
    }

    func testAccessRequestsOfferTheProfileEditorAndNoGrantButton() {
        let request = PermissionRequest(pluginID: "computer", pluginName: "Bot access", tool: "access", summary: "Inbox needs shell access", decision: .pending)
        XCTAssertTrue(request.isAccess)
        XCTAssertFalse(request.isShell)
        XCTAssertEqual(request.choices.map(\.1), ["access", "deny"])
        XCTAssertFalse(request.choices.contains { ["allow", "always"].contains($0.1) })
    }

    func testTheBotProfileCarriesPolicyIntoItsModel() throws {
        let json = """
            {"id":"inbox","name":"Inbox","description":"Read inbox","symbol_name":"envelope","accent":"indigo","runner_id":"runner","provider":"deepseek","created_at":1,"permissions":{"connections":{},"tools":["codemode"],"filesystem":"none","shell":false}}
            """
        let bot = try Wire.decoder.decode(Wire.Bot.self, from: Data(json.utf8)).toModel()
        XCTAssertEqual(bot.permissions?.connections, [:])
        XCTAssertEqual(bot.permissions?.tools, ["codemode"])
        XCTAssertEqual(bot.permissions?.filesystem, "none")
    }
}
