import XCTest
@testable import Lorca

final class ChatMessageLinkTests: XCTestCase {
    func testMessageReferenceStaysInLorca() throws {
        let link = try XCTUnwrap(ChatMessageLink(url: URL(string: "lorca://message?chat_id=chat-1&message_id=msg-2")!))
        XCTAssertEqual(link.chatID, "chat-1")
        XCTAssertEqual(link.messageID, "msg-2")
        let encoded = try XCTUnwrap(ChatMessageLink(url: URL(string: "lorca://message?message_id=msg%2F2&chat_id=chat%20one")!))
        XCTAssertEqual(encoded.chatID, "chat one")
        XCTAssertEqual(encoded.messageID, "msg/2")
    }

    func testMalformedAndExternalSchemesAreRejected() {
        for text in ["https://message?chat_id=c&message_id=m", "lorca://pair?chat_id=c&message_id=m",
            "lorca://message?chat_id=c", "lorca://message?chat_id=&message_id=m",
            "lorca://message?chat_id=c&chat_id=d&message_id=m", "lorca://message/path?chat_id=c&message_id=m",
            "lorca://user@message?chat_id=c&message_id=m", "lorca://message:80?chat_id=c&message_id=m"] {
            XCTAssertNil(ChatMessageLink(url: URL(string: text)!), text)
        }
    }
}
