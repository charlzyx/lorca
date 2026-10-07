import Foundation

/// A link to a locally synced message. The CLI report carries ids; opening it stays inside
/// Lorca and reads any older transcript pages through the local CLI.
struct ChatMessageLink: Equatable {
    static let didOpen = Notification.Name("LorcaChatMessageLinkDidOpen")
    let chatID: String
    let messageID: String

    init?(url: URL) {
        guard let parts = URLComponents(url: url, resolvingAgainstBaseURL: false),
            parts.scheme?.lowercased() == "lorca", parts.host == "message",
            parts.path.isEmpty, parts.port == nil, parts.user == nil, parts.password == nil,
            parts.fragment == nil, let items = parts.queryItems,
            items.count == 2,
            let chat = items.first(where: { $0.name == "chat_id" })?.value, !chat.isEmpty,
            let message = items.first(where: { $0.name == "message_id" })?.value, !message.isEmpty
        else { return nil }
        chatID = chat
        messageID = message
    }
}
