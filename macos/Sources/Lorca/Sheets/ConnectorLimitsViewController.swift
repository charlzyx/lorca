import AppKit

final class ConnectorLimitsViewController: SheetViewController {
    private struct Response: Decodable {
        struct Limits: Decodable { var maxCalls: Int; var windowSecs: Int; var maxConcurrency: Int }
        var limits: Limits
        var activeCalls: Int
        var retryAt: Double?
    }
    private let store = AppStore.shared
    private let pluginID: String
    private let runnerID: Device.ID
    private let scope = NSPopUpButton()
    private let rate = NSTextField()
    private let window = NSTextField()
    private let concurrency = NSTextField()
    private let state = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private var generation = 0

    init(pluginID: String, runner: Device) {
        self.pluginID = pluginID
        self.runnerID = runner.id
        super.init(title: L("Shared connector limits"), subtitle: L("All bots on %@ share these call rates and concurrency limits. Service limits apply across its accounts. Service retry guidance still applies.", runner.name), width: 520)
    }
    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        scope.addItems(withTitles: [L("This account"), L("All accounts for this service")])
        scope.target = self
        scope.action = #selector(loadLimits)
        contentStack.addArrangedSubview(scope)
        let section = SectionView(title: L("Call limits"))
        section.setRows([(L("Calls per window"), rate), (L("Window (seconds)"), window), (L("Concurrent calls"), concurrency)].map { title, field in
            field.alignment = .right
            field.setAccessibilityLabel(title)
            field.translatesAutoresizingMaskIntoConstraints = false
            field.widthAnchor.constraint(equalToConstant: 130).isActive = true
            return Build.stack([Build.label(title, font: .systemFont(ofSize: 12)), NSView(), field], orientation: .horizontal, spacing: 10)
        })
        contentStack.addArrangedSubview(section)
        contentStack.addArrangedSubview(state)
        for view in [section, state] { view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true }
        setButtons(confirm: L("Save"))
        loadLimits()
    }

    private var params: [String: Any] { ["plugin_id": pluginID, "runner_id": runnerID, "scope": scope.indexOfSelectedItem == 1 ? "service" : "account"] }

    @objc private func loadLimits() {
        generation += 1
        let generation = generation
        let params = params
        confirmButton.isEnabled = false
        state.stringValue = L("Loading…")
        Task { [weak self] in
            guard let self else { return }
            do {
                let response = try await store.client.request("connector_limits.get", params, as: Response.self)
                guard generation == self.generation else { return }
                rate.stringValue = String(response.limits.maxCalls)
                window.stringValue = String(response.limits.windowSecs)
                concurrency.stringValue = String(response.limits.maxConcurrency)
                state.stringValue = response.retryAt.map { L("Service cooldown until %@", Format.upcoming(Date(timeIntervalSince1970: $0))) } ?? L("%d calls active. Zero call or concurrency capacity pauses calls.", response.activeCalls)
                confirmButton.isEnabled = true
            } catch {
                guard generation == self.generation else { return }
                state.stringValue = error.localizedDescription
            }
            fitSheetToContent()
        }
    }

    override func confirmTapped() {
        guard let rate = Int(rate.stringValue), (0...1_000_000).contains(rate),
            let window = Int(window.stringValue), (1...86_400).contains(window),
            let concurrency = Int(concurrency.stringValue), (0...256).contains(concurrency)
        else {
            state.stringValue = L("Use whole numbers: up to 1,000,000 calls, a window of 1–86,400 seconds, and up to 256 concurrent calls.")
            fitSheetToContent()
            return
        }
        var params = params
        params["limits"] = ["max_calls": rate, "window_secs": window, "max_concurrency": concurrency]
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do { _ = try await store.client.request("connector_limits.set", params); dismiss(nil) }
            catch { state.stringValue = error.localizedDescription; confirmButton.isEnabled = true; fitSheetToContent() }
        }
    }
}
