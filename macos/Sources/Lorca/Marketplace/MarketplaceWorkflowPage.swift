import AppKit

extension MarketplacePage {
    func workflowSection(_ packs: [WorkflowPack]) -> NSView {
        let rows = packs.map { pack -> MarketplaceRow in
            let icon = NSImageView()
            icon.image = NSImage(systemSymbolName: "point.3.connected.trianglepath.dotted", accessibilityDescription: nil)
            icon.symbolConfiguration = .init(pointSize: 24, weight: .regular)
            icon.contentTintColor = .controlAccentColor
            icon.widthAnchor.constraint(equalToConstant: 40).isActive = true
            let start = ActionButton(title: L("Set Up…")) { [weak self] in self?.market.openWorkflow(pack) }
            start.isEnabled = market.runnerID != nil
            let row = MarketplaceRow(media: icon, title: pack.name, byline: nil, subtitle: pack.outcome, accessory: start)
            row.onOpen = { [weak self] in self?.market.openWorkflow(pack) }
            return row
        }
        return MarketplaceSection(title: L("Guided Workflows"), rows: rows)
    }
}

final class MarketplaceWorkflowCatalogPage: MarketplacePage {
    override func reload() {
        for view in content.arrangedSubviews { content.removeArrangedSubview(view); view.removeFromSuperview() }
        let title = Build.label(L("What would you like to accomplish?"), font: .systemFont(ofSize: 20, weight: .semibold), lines: 0)
        let subtitle = Build.label(L("Choose a workflow, connect its tools, and review a sample before enabling a schedule."), font: .systemFont(ofSize: 13), color: .secondaryLabelColor, lines: 0)
        let section = workflowSection(market.catalog.packs)
        for item in [title, subtitle, section] {
            content.addArrangedSubview(item)
            item.widthAnchor.constraint(equalTo: content.widthAnchor).isActive = true
        }
        content.setCustomSpacing(10, after: title)
        content.setCustomSpacing(24, after: subtitle)
        if market.catalog.packs.isEmpty {
            let label = Build.label(market.loading == .loaded ? L("This CLI has no guided workflows. Update Lorca to add them.") : L("Loading workflows…"), font: .systemFont(ofSize: 13), lines: 0)
            content.addArrangedSubview(label)
            let retry = ActionButton(title: L("Try Again")) { [weak self] in self?.market.load() }
            content.addArrangedSubview(retry)
        }
    }
}

/// A recoverable setup, pinned to its chosen Runner even if the marketplace picker changes.
/// The CLI owns the step state and review gate; closing this page leaves it resumable.
final class MarketplaceWorkflowPage: MarketplacePage {
    private let pack: WorkflowPack
    private let setupRunnerID: Device.ID
    private var progress: WorkflowProgress?
    private var busy = false
    private var editing = false
    private var fields: [String: NSTextField] = [:]
    private var botChoices: [String: WorkflowPopup] = [:]
    private var error: String?
    private var fetching = false
    private var refreshAgain = false

    init(market: MarketplaceViewController, pack: WorkflowPack, runnerID: Device.ID) {
        self.pack = pack
        setupRunnerID = runnerID
        super.init(market: market)
    }

    #if DEBUG
    private var isCaptureFixture = false

    /// Injects only presentation data; the production renderer builds every control.
    convenience init(market: MarketplaceViewController, captureProgress: WorkflowProgress) {
        self.init(market: market, pack: captureProgress.setup.pack, runnerID: captureProgress.setup.runnerId)
        precondition(store.isMock, "UI captures require LORCA_MOCK=1")
        progress = captureProgress
        isCaptureFixture = true
    }
    #endif

    override func viewDidLoad() {
        super.viewDidLoad()
        #if DEBUG
        if isCaptureFixture { render(); return }
        #endif
        store.observe(self) { [weak self] event in
            guard let self, !self.editing else { return }
            switch event {
            case .rosterChanged, .snapshotReplaced, .turnFinished:
                if self.busy || self.fetching { self.refreshAgain = true } else { self.refresh() }
            default: break
            }
        }
        start()
    }

    override func reload() {
        // Preserve fields while typing, including during relay/Runner status changes.
        guard !editing || fields.isEmpty else { return }
        render()
    }

    private func start() {
        perform("start", ["pack_id": pack.id, "runner_id": setupRunnerID])
    }

    private func refresh() {
        guard let id = progress?.setup.id, !fetching, !busy else { return }
        fetching = true
        Task { [weak self] in
            guard let self else { return }
            defer {
                self.fetching = false
                if self.refreshAgain, !self.busy, !self.editing {
                    self.refreshAgain = false
                    self.refresh()
                }
            }
            do {
                self.progress = try await self.store.workflow("get", ["id": id])
                self.error = nil
            } catch { self.error = error.localizedDescription }
            self.render()
        }
    }

    private func perform(_ method: String, _ params: [String: Any]) {
        guard !busy else { return }
        busy = true
        error = nil
        if !editing { render() }
        Task { [weak self] in
            guard let self else { return }
            defer {
                self.busy = false
                self.render()
                if !self.editing && (self.refreshAgain || method == "sample") {
                    self.refreshAgain = false
                    self.refresh()
                }
            }
            do {
                self.progress = try await self.store.workflow(method, params)
                self.editing = false
                self.fields.removeAll()
                self.botChoices.removeAll()
            } catch { self.error = error.localizedDescription }
        }
    }

    private func add(_ view: NSView, spacing: CGFloat = 14) {
        content.addArrangedSubview(view)
        view.widthAnchor.constraint(equalTo: content.widthAnchor).isActive = true
        content.setCustomSpacing(spacing, after: view)
    }

    private func text(_ value: String, color: NSColor = .secondaryLabelColor) -> NSTextField {
        Build.label(value, font: .systemFont(ofSize: 13), color: color, lines: 0)
    }

    private func action(_ title: String, enabled: Bool = true, run: @escaping () -> Void) -> ActionButton {
        let button = ActionButton(title: title, action: run)
        button.isEnabled = enabled && !busy
        return button
    }

    private func render() {
        // Save unfinished form text across errors and in-place rerenders.
        let drafts = fields.mapValues(\.stringValue)
        let picked = botChoices.compactMapValues { $0.selectedItem?.representedObject as? String }
        fields.removeAll()
        botChoices.removeAll()
        for view in content.arrangedSubviews { content.removeArrangedSubview(view); view.removeFromSuperview() }
        add(Build.label(pack.name, font: .systemFont(ofSize: 20, weight: .semibold)), spacing: 8)
        add(text(pack.outcome), spacing: 10)
        let runnerName = store.device(setupRunnerID)?.name ?? setupRunnerID
        add(text(L("Runs on %@. Closing this page saves your progress.", runnerName)), spacing: 22)
        if let error { add(text(error, color: .systemRed)) }
        guard let progress else {
            add(text(busy ? L("Loading setup…") : L("Setup is unavailable right now.")))
            add(action(L("Try Again")) { [weak self] in self?.start() })
            return
        }
        let setup = progress.setup
        if setup.phase == "cancelled" {
            add(text(L("Setup is cancelled. Its specialists and connections are kept for reuse, and its imported routines are paused.")))
            add(action(L("Resume Setup")) { [weak self] in self?.start() })
            return
        }
        if setup.phase == "questions" || editing {
            editing = true
            add(text(L("1 · Answer the questions for this workflow"), color: .labelColor))
            for question in setup.pack.questions {
                add(Build.label(question.label, font: .systemFont(ofSize: 13, weight: .medium)), spacing: 6)
                let field = NSTextField(string: drafts[question.id] ?? setup.answers[question.id] ?? "")
                field.placeholderString = question.placeholder
                field.setAccessibilityLabel(question.label)
                fields[question.id] = field
                add(field)
            }
            if !progress.specialists.isEmpty {
                add(text(L("Specialists · reuse a bot on this Runner or add a suitable one.")))
            }
            for specialist in progress.specialists {
                let popup = WorkflowPopup()
                popup.addItem(withTitle: L("Reuse a suitable bot or add %@", specialist.name))
                for bot in specialist.choices {
                    popup.addItem(withTitle: bot.name)
                    popup.lastItem?.representedObject = bot.id
                }
                if let id = picked[specialist.id] ?? specialist.selectedId,
                   let item = popup.itemArray.first(where: { $0.representedObject as? String == id }) { popup.select(item) }
                popup.setAccessibilityLabel(specialist.name)
                botChoices[specialist.id] = popup
                add(popup)
            }
            add(action(L("Continue")) { [weak self] in
                guard let self else { return }
                let answers = self.fields.mapValues(\.stringValue)
                let bots = self.botChoices.compactMapValues { $0.selectedItem?.representedObject as? String }
                self.perform("configure", ["id": setup.id, "answers": answers, "bot_ids": bots])
            })
        } else {
            let connected = progress.connections.filter { $0.state == "ready" }.count
            add(text(L("2 · Connections · %d of %d ready", connected, progress.connections.count), color: .labelColor))
            for connection in progress.connections { renderConnection(connection, setupID: setup.id) }
            if let blocked = progress.blockedReason { add(text(blocked, color: .systemOrange)) }
            add(text(L("3 · Run a sample and review the result"), color: .labelColor))
            if progress.isRunning {
                let spinner = NSProgressIndicator()
                spinner.style = .spinning
                spinner.startAnimation(nil)
                add(Build.stack([spinner, text(L("Running the sample…"))], orientation: .horizontal, spacing: 10))
            } else {
                if let sample = setup.sample {
                    if let error = sample.error { add(text(error, color: .systemRed)) }
                    if sample.state == "running" { add(text(L("The sample was interrupted. Run it again to produce a result."))) }
                    for message in progress.sampleMessages {
                        if let value = message.body.text { add(text(value, color: .labelColor)) }
                    }
                    if sample.state == "ready", !progress.sampleMessages.isEmpty {
                        add(action(L("I Have Reviewed This Result")) { [weak self] in
                            self?.perform("review", ["id": setup.id, "job_id": sample.jobId])
                        })
                    }
                }
                add(action(setup.sample == nil ? L("Run Sample") : L("Run Another Sample"), enabled: progress.canSample) { [weak self] in
                    self?.perform("sample", ["id": setup.id])
                })
            }
            // Schedules are offered only after explicit review of a completed sample.
            if progress.canEnable {
                add(text(L("4 · Choose whether to enable the schedule"), color: .labelColor))
                for routine in progress.routines {
                    add(text("\(routine.name) · \(routine.scheduleText) · " + (routine.isEnabled ? L("Enabled") : L("Paused"))))
                }
                if setup.phase != "enabled" {
                    add(action(L("Enable Schedules")) { [weak self] in self?.perform("enable", ["id": setup.id]) })
                    add(action(L("Finish with Schedules Paused")) { [weak self] in self?.market.finishWorkflow(chatID: setup.sample?.chatId) })
                } else {
                    add(text(L("This workflow's schedules are enabled."), color: .systemGreen))
                    add(action(L("Open Workflow Chat")) { [weak self] in self?.market.finishWorkflow(chatID: setup.sample?.chatId) })
                }
            } else if setup.sample?.state != "ready" {
                add(text(L("Imported routines stay paused until you review a sample and enable them.")))
            }
            if setup.phase != "enabled" {
                add(action(L("Edit Setup"), enabled: !progress.isRunning) { [weak self] in self?.editing = true; self?.render() })
            }
            add(action(L("Refresh Progress")) { [weak self] in self?.refresh() })
        }
        add(action(setup.phase == "enabled" ? L("Cancel Setup and Pause Imported Routines") : L("Cancel Setup")) { [weak self] in
            self?.perform("cancel", ["id": setup.id])
        })
    }

    private func renderConnection(_ connection: WorkflowProgress.Connection, setupID: String) {
        add(Build.label(connection.name, font: .systemFont(ofSize: 13, weight: .semibold)), spacing: 6)
        add(text(connection.detail, color: connection.state == "ready" ? .systemGreen : .secondaryLabelColor), spacing: 6)
        if !connection.choices.isEmpty {
            let popup = WorkflowPopup()
            popup.addItem(withTitle: L("Choose an account…"))
            for choice in connection.choices {
                popup.addItem(withTitle: choice.label)
                popup.lastItem?.representedObject = choice.id
            }
            if let item = popup.itemArray.first(where: { $0.representedObject as? String == connection.selectedId }) { popup.select(item) }
            popup.setAccessibilityLabel(L("%@ account", connection.name))
            popup.onChange = { [weak self, weak popup] in
                guard let id = popup?.selectedItem?.representedObject as? String else { return }
                self?.perform("connection", ["id": setupID, "service_id": connection.serviceId, "plugin_id": id])
            }
            popup.isEnabled = !busy && progress?.isRunning != true && progress?.setup.phase != "enabled"
            add(popup)
        } else if connection.available, connection.selectedId == nil {
            add(action(L("Add %@", connection.name), enabled: progress?.setup.phase != "enabled") { [weak self] in
                self?.perform("connection", ["id": setupID, "service_id": connection.serviceId, "account_name": self?.pack.name ?? "Workflow"])
            })
        } else if !connection.available {
            add(text(L("This integration is unavailable in the current marketplace. Update Lorca or the marketplace, then resume this saved setup."), color: .systemOrange))
        }
        if connection.selectedId != nil, !connection.choices.contains(where: { $0.id == connection.selectedId }) {
            add(action(L("Clear Removed Account Selection"), enabled: progress?.setup.phase != "enabled") { [weak self] in
                self?.perform("clear_connection", ["id": setupID, "service_id": connection.serviceId])
            })
        }
        if let id = connection.selectedId, connection.state != "ready" {
            add(action(connection.state == "needs_setup" ? L("Set Up…") : L("Sign In…")) { [weak self] in
                guard let self, let runner = self.store.device(self.setupRunnerID) else { return }
                PluginViewController.present(pluginID: id, runner: runner, bot: nil, from: self)
            })
        }
    }
}

private final class WorkflowPopup: NSPopUpButton {
    var onChange: (() -> Void)?
    init() {
        super.init(frame: .zero, pullsDown: false)
        target = self
        action = #selector(changed)
    }
    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
    @objc private func changed() { onChange?() }
}
