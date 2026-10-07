import XCTest
@testable import Lorca

@MainActor
final class BudgetTests: XCTestCase {
    private func usage() -> ChatUsage {
        ChatUsage(contextTokens: 0, contextWindow: 0, inputTokens: 0, outputTokens: 0,
                  cacheReadTokens: 0, costUSD: 0, turns: 1, model: "test")
    }

    func testUnknownAndLegacyPricingNeverReadAsFree() {
        let legacy = usage()
        XCTAssertTrue(legacy.spendSummary.contains(L("Pricing unknown · %d turns", 1)))
        var unknown = usage()
        unknown.pricedCalls = 1
        unknown.unknownPriceCalls = 1
        unknown.pricingKinds = ["unknown"]
        XCTAssertTrue(unknown.spendSummary.contains(L("Pricing unknown")))
        XCTAssertFalse(unknown.spendSummary.contains("$0.00"))
    }

    func testZeroSubscriptionEstimateKeepsItsSubscriptionLabel() {
        var subscription = usage()
        subscription.pricedCalls = 1
        subscription.pricingKinds = ["subscription_estimate"]
        XCTAssertTrue(subscription.spendSummary.contains(L("API-equivalent estimate %@", "$0.00")))
        var api = usage()
        api.pricedCalls = 1
        api.pricingKinds = ["api"]
        XCTAssertTrue(api.spendSummary.contains(L("API %@", "$0.00")))
    }

    func testEncryptedRunnerProjectionPreservesEventRecoveryAndUnknownPricing() throws {
        let data = Data(#"{"kind":"job","id":"job-1","runner_id":"runner","bot_id":"bot","chat_id":"chat","job_kind":"event","limits":{"max_tokens":100,"max_runtime_secs":60},"usage":{"tokens":100,"api_cost_usd":0,"subscription_estimate_usd":0.2,"unknown_price_calls":3,"estimated_calls":1,"model_calls":4,"runtime_secs":20,"retries":1,"connector_calls":2},"state":"budget_exhausted","reason":"Increase the allowance","updated_at":1}"#.utf8)
        let state = try Wire.decoder.decode(BudgetState.self, from: data)
        XCTAssertTrue(state.needsRecovery)
        XCTAssertEqual(state.jobKind, "event")
        XCTAssertNil(state.taskId)
        XCTAssertEqual(state.usage.unknownPriceCalls, 3)
        XCTAssertEqual(state.limits.maxRuntimeSecs, 60)
    }
}
