import Foundation

struct BrowserSession: Decodable, Identifiable, Equatable {
    let id: String
    let botID: String
    let runnerID: String
    let account: String
    let profile: String
    let state: String
    let selected: Bool
    let revision: UInt64

    enum CodingKeys: String, CodingKey {
        case id, account, profile, state, selected, revision
        case botID = "bot_id", runnerID = "runner_id"
    }

    var title: String { "\(account) · \(profile)" }
    var isHuman: Bool { state == "human" }
    var isStopped: Bool { state == "stopped" }
    var isTakingOver: Bool { state == "taking_over" }
    var canReturnToBot: Bool { isHuman }
}

struct BrowserCapabilities: Decodable {
    let visibleOpen: Bool
    let localInput: Bool
    let remoteLiveView: Bool
    let remoteInput: Bool
    let nativeInput: Bool

    enum CodingKeys: String, CodingKey {
        case visibleOpen = "visible_open", localInput = "local_input"
        case remoteLiveView = "remote_live_view", remoteInput = "remote_input", nativeInput = "native_input"
    }
}

struct BrowserSessionList: Decodable {
    let sessions: [BrowserSession]
    let capabilities: BrowserCapabilities
}
