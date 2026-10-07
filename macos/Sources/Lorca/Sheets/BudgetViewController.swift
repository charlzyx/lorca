import AppKit

/// User-owned allowances are changed through the local CLI and managed by the bot's Runner.
final class BudgetViewController: SheetViewController {
    private let store = AppStore.shared
    private let bot: Bot
    private let chatID: Chat.ID
    private let routineID: Routine.ID?
    private let taskID: String?
    private let scope = NSPopUpButton()
    private var targets: [(kind: String, id: String)] = []
    private var fields: [String: NSTextField] = [:]
    private let stateSection = SectionView(title: L("Usage"))
    private let note = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let resume = NSButton(title: L("Save and resume"), target: nil, action: nil)
    private let renew = NSButton(title: L("Renew allowance and resume…"), target: nil, action: nil)
    private let errorLabel = Build.label("", font: .systemFont(ofSize: 12), color: .systemRed, lines: 0)
    private var loadedBudget: BudgetState?
    private var loadGeneration = 0
    private var canEdit = false

    init(bot: Bot, chatID: Chat.ID, routineID: Routine.ID? = nil, taskID: String? = nil) {
        self.bot = bot
        self.chatID = chatID
        self.routineID = routineID
        self.taskID = taskID
        super.init(title: L("Budget limits"), subtitle: L("Limits are enforced on %@'s Runner. Leave a field empty for unlimited. Runtime includes checks, retries, review, and waiting.", bot.name), width: 560)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        if let routineID {
            targets = [("routine", routineID)]
            scope.addItem(withTitle: store.routine(routineID)?.name ?? L("Routine"))
        } else if let taskID {
            targets = [("task", taskID)]
            scope.addItem(withTitle: L("Task allowance"))
        } else {
            targets = [("chat", chatID)]
            scope.addItem(withTitle: L("Allowance for new tasks in this chat"))
            let history = store.budgets(for: chatID, runnerID: bot.runnerID).filter { $0.kind == "job" || $0.kind == "task" }.prefix(12)
            for budget in history {
                targets.append((budget.kind, budget.id))
                scope.addItem(withTitle: "\(budget.stateLabel) · \(String(budget.id.suffix(8)))")
            }
            if let recovery = history.firstIndex(where: { $0.needsRecovery }) { scope.selectItem(at: recovery + 1) }
        }
        scope.target = self
        scope.action = #selector(scopeChanged)
        contentStack.addArrangedSubview(scope)
        contentStack.addArrangedSubview(stateSection)
        let limits = SectionView(title: L("Allowance"))
        let specs = [
            ("max_usd", L("Spending (USD)")), ("max_tokens", L("Total tokens")),
            ("max_runtime_secs", L("Runtime (seconds)")), ("max_retries", L("Retries")),
            ("max_connector_calls", L("Connector calls")),
        ]
        limits.setRows(specs.map { key, title in
            let label = Build.label(title, font: .systemFont(ofSize: 12))
            let field = NSTextField()
            field.placeholderString = L("Unlimited")
            field.setAccessibilityLabel(title)
            field.alignment = .right
            field.translatesAutoresizingMaskIntoConstraints = false
            field.widthAnchor.constraint(equalToConstant: 160).isActive = true
            fields[key] = field
            return Build.stack([label, NSView(), field], orientation: .horizontal, spacing: 10)
        })
        contentStack.addArrangedSubview(limits)
        contentStack.addArrangedSubview(note)
        for button in [resume, renew] {
            button.bezelStyle = .rounded
            button.target = self
        }
        resume.action = #selector(resumeWork)
        renew.action = #selector(renewWork)
        let actions = Build.stack([resume, renew], orientation: .horizontal, spacing: 8)
        contentStack.addArrangedSubview(actions)
        contentStack.addArrangedSubview(errorLabel)
        for view in [stateSection, limits, note, actions, errorLabel] {
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        errorLabel.isHidden = true
        setButtons(confirm: L("Save"))
        scopeChanged()
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.refreshUsage()
            default: break
            }
        }
    }

    private var target: (kind: String, id: String) { targets[max(0, scope.indexOfSelectedItem)] }
    private var budget: BudgetState? {
        let stored = store.budgets.first { $0.kind == target.kind && $0.id == target.id && $0.runnerId == bot.runnerID }
        if let loadedBudget, loadedBudget.kind == target.kind, loadedBudget.id == target.id,
           stored == nil || loadedBudget.updatedAt >= (stored?.updatedAt ?? 0) { return loadedBudget }
        return stored
    }

    @objc private func scopeChanged() {
        canEdit = false
        loadedBudget = nil
        showFields()
        refreshUsage()
        loadGeneration += 1
        let generation = loadGeneration
        let target = target
        Task { [weak self] in
            guard let self else { return }
            struct Response: Decodable { var budgets: [BudgetState] }
            do {
                let response = try await store.client.request("budgets.list", ["runner_id": bot.runnerID], as: Response.self)
                guard generation == loadGeneration else { return }
                loadedBudget = response.budgets.first { $0.kind == target.kind && $0.id == target.id }
                canEdit = true
                showFields()
                refreshUsage()
            } catch {
                guard generation == loadGeneration else { return }
                errorLabel.stringValue = error.localizedDescription
                errorLabel.isHidden = false
                refreshUsage()
            }
        }
    }

    private func showFields() {
        let limits = budget?.limits
        fields["max_usd"]?.stringValue = limits?.maxUsd.map { String($0) } ?? ""
        fields["max_tokens"]?.stringValue = limits?.maxTokens.map(String.init) ?? ""
        fields["max_runtime_secs"]?.stringValue = limits?.maxRuntimeSecs.map(String.init) ?? ""
        fields["max_retries"]?.stringValue = limits?.maxRetries.map(String.init) ?? ""
        fields["max_connector_calls"]?.stringValue = limits?.maxConnectorCalls.map(String.init) ?? ""
    }

    private func refreshUsage() {
        guard isViewLoaded else { return }
        if let budget {
            let usage = budget.usage
            stateSection.setRows([
                KeyValueRow(key: L("State"), value: budget.stateLabel, tint: budget.needsRecovery ? .systemOrange : .labelColor),
                KeyValueRow(key: L("API spending"), value: String(format: "$%.4f", usage.apiCostUsd)),
                KeyValueRow(key: L("Subscription API-equivalent estimate"), value: String(format: "$%.4f", usage.subscriptionEstimateUsd)),
                KeyValueRow(key: L("Unknown pricing"), value: L("%d calls", usage.unknownPriceCalls)),
                KeyValueRow(key: L("Tokens / runtime"), value: "\(Format.tokens(usage.tokens)) · \(Int(usage.runtimeSecs)) s"),
                KeyValueRow(key: L("Retries / connector calls"), value: "\(usage.retries) / \(usage.connectorCalls)"),
            ])
            note.stringValue = budget.reason ?? L("Requests without reported usage use token and cost estimates. Unknown prices need a token or runtime allowance; they are never treated as free.")
            resume.isEnabled = canEdit && budget.state != "running" && target.kind != "chat"
            renew.isEnabled = resume.isEnabled
        } else {
            stateSection.setRows([KeyValueRow(key: L("State"), value: L("No limits configured"))])
            note.stringValue = L("API spending and subscription API-equivalent estimates count toward the spending allowance. For unknown pricing, use a token or runtime allowance.")
            resume.isEnabled = canEdit && (target.kind == "routine" || target.kind == "task")
            renew.isEnabled = resume.isEnabled
        }
        confirmButton.isEnabled = canEdit
        for field in fields.values { field.isEnabled = canEdit }
        fitSheetToContent()
    }

    private func values() throws -> [String: Any] {
        var values: [String: Any] = [:]
        for (key, field) in fields {
            let text = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !text.isEmpty else { continue }
            if key == "max_usd", let value = Double(text), value.isFinite, value >= 0 { values[key] = value }
            else if key != "max_usd", let value = Int(text), value >= 0 { values[key] = value }
            else { throw NSError(domain: "Budget", code: 1, userInfo: [NSLocalizedDescriptionKey: L("Use a nonnegative number for each limit, or leave it empty.")]) }
        }
        return values
    }

    override func confirmTapped() { save(resuming: false, renewing: false) }
    @objc private func resumeWork() { save(resuming: true, renewing: false) }
    @objc private func renewWork() {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Renew this allowance?")
        alert.informativeText = L("This grants the full configured allowance again and resumes from the existing transcript. Check completed effects before resuming interrupted work.")
        alert.addButton(withTitle: L("Renew and resume"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { [weak self] response in
            if response == .alertFirstButtonReturn { self?.save(resuming: true, renewing: true) }
        }
    }

    private func save(resuming: Bool, renewing: Bool) {
        do {
            let ownedAdmission = budget?.jobKind == "event" || budget?.taskId != nil || target.kind == "task"
            let limits = try values()
            let params: [String: Any] = ["kind": target.kind, "id": target.id, "bot_id": bot.id, "chat_id": chatID, "runner_id": bot.runnerID, "limits": limits]
            canEdit = false
            scope.isEnabled = false
            confirmButton.isEnabled = false
            resume.isEnabled = false
            renew.isEnabled = false
            Task { [weak self] in
                guard let self else { return }
                do {
                    _ = try await store.client.request("budgets.set", params)
                    if resuming {
                        var recovery = params
                        recovery["renew"] = renewing
                        recovery["run"] = !ownedAdmission
                        recovery["request_id"] = UUID().uuidString
                        _ = try await store.client.request("budgets.resume", recovery)
                    }
                    if resuming && ownedAdmission {
                        canEdit = true
                        scope.isEnabled = true
                        errorLabel.textColor = .secondaryLabelColor
                        errorLabel.stringValue = L("Allowance recovered. Retry the delivery in Events or run the task again in Tasks so its ownership and inbox admission are checked.")
                        errorLabel.isHidden = false
                        confirmButton.isEnabled = true
                        refreshUsage()
                        return
                    }
                    dismiss(nil)
                } catch {
                    canEdit = true
                    scope.isEnabled = true
                    errorLabel.textColor = .systemRed
                    errorLabel.stringValue = error.localizedDescription
                    errorLabel.isHidden = false
                    confirmButton.isEnabled = true
                    refreshUsage()
                }
            }
        } catch {
            errorLabel.stringValue = error.localizedDescription
            errorLabel.isHidden = false
            fitSheetToContent()
        }
    }
}
