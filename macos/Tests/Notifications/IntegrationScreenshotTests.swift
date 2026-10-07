import AppKit
import XCTest
@testable import Lorca

/// Opt-in evidence capture of the production controllers. The companion read-only fixture
/// supplies wire statuses, never tokens, chats, or live provider authorization.
@MainActor
final class IntegrationScreenshotTests: XCTestCase {
    func testCaptureIntegrationScreens() async throws {
        guard let folder = ProcessInfo.processInfo.environment["LORCA_CAPTURE_71_DIR"],
              let port = Int(ProcessInfo.processInfo.environment["LORCA_PORT"] ?? ""),
              ![4862, 4863].contains(port) else {
            throw XCTSkip("Run evidence/issue-71/capture.ts for native fixture screenshots.")
        }
        let (data, _) = try await URLSession.shared.data(from: URL(string: "http://127.0.0.1:\(port)/capture-fixture")!)
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        guard fixture["issue"] as? Int == 71 else { throw CLIClient.RequestError(message: "This is not the issue 71 screenshot fixture") }
        let store = AppStore.shared
        guard !store.isMock, store.client.url.port == port else { throw CLIClient.RequestError(message: "The capture requires its isolated fixture connection") }
        _ = NSApplication.shared
        store.start()
        defer { store.stop() }
        try await wait { store.isConnected && store.device("fixture-runner") != nil }
        guard store.chats.isEmpty, store.providers.isEmpty else { throw CLIClient.RequestError(message: "Screenshot fixtures must contain no chats or provider credentials") }
        let runner = try XCTUnwrap(store.device("fixture-runner"))
        let directory = URL(fileURLWithPath: folder, isDirectory: true)

        let marketplace = MarketplaceViewController(runnerID: runner.id, size: NSSize(width: 800, height: 300), onOpenChat: { _ in })
        _ = marketplace.view
        try await wait { marketplace.loading == .loaded }
        marketplace.openList(title: "Integrations", items: { catalog in catalog.plugins.map { .plugin($0) } })
        try await wait { self.texts(marketplace.view).contains("Integrations") && !self.texts(marketplace.view).contains("Featured Plugins") }
        try capture(marketplace, to: directory.appendingPathComponent("01-marketplace.png"))

        let accounts = PluginAccountsViewController(serviceID: "gmail", name: "Gmail", runner: runner)
        _ = accounts.view
        XCTAssertTrue(texts(accounts.view).contains("Add Account…"))
        try capture(accounts, to: directory.appendingPathComponent("02-named-accounts.png"))

        for (label, readyText, filename) in [
            ("Work", "Connected", "03-connected-account.png"),
            ("Personal", "Not signed in", "04-needs-sign-in.png"),
            ("Shared", "Sign in again to grant the required access", "05-insufficient-access.png"),
        ] {
            let plugin = try XCTUnwrap(runner.plugins.first { $0.serviceID == "gmail" && $0.accountName == label })
            let sheet = PluginViewController(pluginID: plugin.id, runner: runner, bot: nil)
            _ = sheet.view
            try await wait { self.texts(sheet.view).contains(readyText) }
            XCTAssertTrue(texts(sheet.view).contains("Manage Accounts…"))
            try capture(sheet, to: directory.appendingPathComponent(filename))
        }
        let slack = try XCTUnwrap(runner.plugins.first { $0.serviceID == "slack" })
        let errorSheet = PluginViewController(pluginID: slack.id, runner: runner, bot: nil)
        _ = errorSheet.view
        try await wait { self.texts(errorSheet.view).contains("Unable to reach the service. Try again.") }
        try capture(errorSheet, to: directory.appendingPathComponent("06-slack-error.png"))
    }

    private func wait(until predicate: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(15)
        while !predicate() {
            guard Date() < deadline else { throw CLIClient.RequestError(message: "Fixture UI did not finish loading") }
            try await Task.sleep(nanoseconds: 20_000_000)
        }
    }

    private func texts(_ view: NSView) -> [String] {
        let own = (view as? NSTextField).map { [$0.stringValue] } ?? (view as? NSButton).map { [$0.title] } ?? []
        return own + view.subviews.flatMap(texts)
    }

    private func capture(_ controller: NSViewController, to url: URL) throws {
        let view = controller.view
        let size = view.fittingSize
        XCTAssertGreaterThan(size.width, 400)
        XCTAssertGreaterThan(size.height, 150)
        let window = NSWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: .borderless, backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.appearance = NSAppearance(named: .aqua)
        window.contentViewController = controller
        window.setContentSize(size)
        view.wantsLayer = true
        view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        view.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bounds = view.bounds
        let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: bounds))
        view.cacheDisplay(in: bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try png.write(to: url)
        window.orderOut(nil)
    }
}
