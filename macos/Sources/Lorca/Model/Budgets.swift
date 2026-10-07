import Foundation

struct BudgetLimits: Decodable, Hashable {
    var maxUsd: Double?
    var maxTokens: Int?
    var maxRuntimeSecs: Int?
    var maxRetries: Int?
    var maxConnectorCalls: Int?
}

struct BudgetState: Decodable, Hashable {
    struct Usage: Decodable, Hashable {
        var tokens: Int
        var apiCostUsd: Double
        var subscriptionEstimateUsd: Double
        var unknownPriceCalls: Int
        var estimatedCalls: Int
        var modelCalls: Int
        var runtimeSecs: Double
        var retries: Int
        var connectorCalls: Int
    }
    var kind: String
    var id: String
    var runnerId: String
    var botId: String
    var chatId: String
    var jobKind: String?
    var taskId: String?
    var limits: BudgetLimits
    var usage: Usage
    var state: String
    var reason: String?
    var updatedAt: Double

    var needsRecovery: Bool { state == "budget_exhausted" || state == "interrupted" }
    var stateLabel: String {
        switch state {
        case "budget_exhausted": L("Budget exhausted")
        case "interrupted": L("Interrupted — resume explicitly")
        case "running": L("Running…")
        case "complete": L("Finished")
        default: L("Ready")
        }
    }
}

extension AppStore {
    func budgets(for chatID: Chat.ID, runnerID: Device.ID) -> [BudgetState] {
        budgets.filter { $0.chatId == chatID && $0.runnerId == runnerID }.sorted { $0.updatedAt > $1.updatedAt }
    }
}
