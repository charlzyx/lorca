import AppKit

/// One routine's details, after Grok Bot's routine page: the schedule and its next and last
/// runs, the task, the check when it has one, and the actions on it. Run Now starts it on the
/// bot's Runner; Pause and Resume flip the switch the inspector shows; Edit in Chat hands the bot
/// the change to make, since the bot owns its routines; Delete asks first.
final class RoutineViewController: SheetViewController {
    private let store = AppStore.shared
    private let routineID: Routine.ID
    private let bot: Bot

    private let schedule = SectionView(title: L("Schedule"))
    private let task = SectionView(title: L("Task"))
    private let prompt = NSTextView()
    private let checkSection = SectionView(title: L("Check"))
    private let check = NSTextView()
    private let healthSection = SectionView(title: L("Availability and checks"))
    private let runButton = NSButton()
    private let pauseButton = NSButton()
    private let editButton = NSButton()
    private let deleteButton = NSButton()

    /// Called with the text to put in the composer when the user wants the bot to edit it.
    var onEditInChat: ((String) -> Void)?

    init(routineID: Routine.ID, bot: Bot) {
        self.routineID = routineID
        self.bot = bot
        let routine = AppStore.shared.routine(routineID)
        super.init(
            title: routine?.name ?? L("Routine"),
            subtitle: L("A task %@ runs on its own in this chat. %@ set it up and can change it: ask in chat.", bot.name, bot.name),
            width: 520
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        let scroll = Self.readOnly(prompt, font: .systemFont(ofSize: 12))
        task.setRows([scroll])
        let checkScroll = Self.readOnly(check, font: .monospacedSystemFont(ofSize: 11, weight: .regular))
        checkSection.setRows([checkScroll])

        for (button, title, action) in [
            (runButton, L("Run Now"), #selector(runNow)),
            (pauseButton, L("Pause"), #selector(togglePaused)),
            (editButton, L("Edit in Chat…"), #selector(editInChat)),
            (deleteButton, L("Delete…"), #selector(confirmDelete)),
        ] {
            button.title = title
            button.bezelStyle = .rounded
            button.controlSize = .regular
            button.target = self
            button.action = action
        }
        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let actions = Build.stack([runButton, pauseButton, editButton, spacer, deleteButton], orientation: .horizontal, spacing: 8)

        // Details grow with health/recovery notes; keep actions visible on smaller screens.
        let details = Build.stack([schedule, healthSection, task, checkSection], spacing: 12)
        details.alignment = .leading
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(details)
        let detailsScroll = NSScrollView()
        detailsScroll.documentView = document
        detailsScroll.drawsBackground = false
        detailsScroll.hasVerticalScroller = true
        detailsScroll.autohidesScrollers = true
        detailsScroll.translatesAutoresizingMaskIntoConstraints = false
        contentStack.addArrangedSubview(detailsScroll)
        contentStack.addArrangedSubview(actions)
        NSLayoutConstraint.activate([
            detailsScroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            detailsScroll.heightAnchor.constraint(equalToConstant: max(240, min(480, (NSScreen.main?.visibleFrame.height ?? 800) - 260))),
            document.widthAnchor.constraint(equalTo: detailsScroll.contentView.widthAnchor),
            details.topAnchor.constraint(equalTo: document.topAnchor),
            details.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            details.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            details.bottomAnchor.constraint(equalTo: document.bottomAnchor),
            schedule.widthAnchor.constraint(equalTo: details.widthAnchor),
            healthSection.widthAnchor.constraint(equalTo: details.widthAnchor),
            task.widthAnchor.constraint(equalTo: details.widthAnchor),
            checkSection.widthAnchor.constraint(equalTo: details.widthAnchor),
            actions.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 96),
            checkScroll.heightAnchor.constraint(equalToConstant: 120),
        ])

        setButtons(confirm: L("Done"), cancel: nil)
        refresh()
    }

    /// A text view that shows text to read and select, scrolling inside a fixed height.
    private static func readOnly(_ text: NSTextView, font: NSFont) -> NSScrollView {
        text.isEditable = false
        text.isRichText = false
        text.font = font
        text.textColor = .labelColor
        text.drawsBackground = false
        text.textContainerInset = NSSize(width: 8, height: 8)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        let scroll = NSScrollView()
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        return scroll
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .chatsChanged, .snapshotReplaced:
                self?.refresh()
            default:
                break
            }
        }
    }

    /// Redraws from the store; the sheet closes when the routine is gone.
    private func refresh() {
        guard isViewLoaded else { return }
        guard let routine = store.routines(for: bot.id).first(where: { $0.id == routineID }) else {
            dismiss(nil)
            return
        }
        let tint: NSColor = ["failed", "blocked"].contains(routine.state) ? .systemOrange : .secondaryLabelColor
        let scheduleRow = KeyValueRow(key: L("Schedule"), value: routine.scheduleText)
        scheduleRow.toolTip = routine.schedule
        let timezoneRow = ActionRow(key: L("Timezone"), value: routine.timezone, tint: .labelColor, actionTitle: L("Change…"))
        timezoneRow.onAction = { [weak self] in self?.changeTimezone() }
        let policyRow = PopUpRow(key: L("Missed runs"), items: [L("Run once"), L("Skip")], selected: routine.missedRunPolicy == "skip" ? 1 : 0)
        policyRow.onChange = { [weak self] index in
            self?.savePolicy(missedRunPolicy: index == 1 ? "skip" : "coalesce")
        }
        schedule.setRows([
            KeyValueRow(key: L("State"), value: routine.stateText, tint: tint),
            scheduleRow,
            timezoneRow,
            policyRow,
            NoteRow(text: routine.missedRunPolicy == "skip" ? L("Skip occurrences more than a minute late. The next occurrence keeps the chosen timezone.") : L("After an outage, run once with current data. Missed occurrences never queue a burst of runs.")),
            KeyValueRow(key: routine.check == nil ? L("Next run") : L("Next check"), value: routine.nextSummary),
            KeyValueRow(key: L("Last run"), value: routine.lastRunSummary),
        ])
        let runner = store.device(bot.runnerID)
        var healthRows: [NSView] = [KeyValueRow(key: L("Runner"), value: runner?.name ?? bot.runnerID),
            KeyValueRow(key: L("Availability"), value: routine.runnerAvailable ? L("Available") : L("Waiting for Runner"))]
        if routine.check != nil {
            healthRows += [KeyValueRow(key: L("Last check"), value: routine.lastCheckAt.map { Format.daySeparator($0) } ?? L("Never")),
                KeyValueRow(key: L("Last successful check"), value: routine.lastSuccessfulCheckAt.map { Format.daySeparator($0) } ?? L("Never"))]
        }
        if let retry = routine.retryAt, routine.isEnabled {
            healthRows.append(KeyValueRow(key: L("Retry after"), value: Format.daySeparator(retry)))
        }
        if let recovery = routine.recoveryAction { healthRows.append(NoteRow(text: recovery)) }
        healthSection.setRows(healthRows)
        if prompt.string != routine.prompt { prompt.string = routine.prompt }
        checkSection.isHidden = routine.check == nil
        if check.string != (routine.check ?? "") { check.string = routine.check ?? "" }
        pauseButton.title = routine.isEnabled ? L("Pause") : L("Resume")
        runButton.isEnabled = !routine.isRunning && routine.runnerAvailable && routine.pausedReason != "authentication"
        runButton.toolTip = runner.map { L("Runs on %@ now", $0.name) } ?? L("Runs on the bot's Runner now")
        fitSheetToContent()
    }

    private func changeTimezone() {
        guard let routine = store.routine(routineID), let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Routine timezone")
        alert.informativeText = L("Use an IANA timezone such as America/New_York, Asia/Singapore, or UTC. Cron follows its daylight-saving changes; intervals count elapsed time.")
        let field = NSTextField(string: routine.timezone)
        field.frame = NSRect(x: 0, y: 0, width: 320, height: 24)
        alert.accessoryView = field
        alert.addButton(withTitle: L("Save"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { [weak self] response in
            if response == .alertFirstButtonReturn { self?.savePolicy(timezone: field.stringValue) }
        }
        alert.window.initialFirstResponder = field
    }

    private func savePolicy(timezone: String? = nil, missedRunPolicy: String? = nil) {
        Task { @MainActor in
            do {
                try await store.setRoutinePolicy(routineID, timezone: timezone, missedRunPolicy: missedRunPolicy)
            } catch {
                let alert = NSAlert()
                alert.messageText = L("Couldn’t update routine")
                alert.informativeText = error.localizedDescription
                if let window = view.window { alert.beginSheetModal(for: window) { _ in } }
                refresh()
            }
        }
    }

    @objc private func runNow() {
        store.runRoutine(routineID)
    }

    @objc private func togglePaused() {
        guard let routine = store.routine(routineID) else { return }
        store.setRoutineEnabled(routineID, !routine.isEnabled)
    }

    @objc private func editInChat() {
        guard let routine = store.routine(routineID) else { return }
        onEditInChat?(L("Edit my routine \"%@\": ", routine.name))
        dismiss(nil)
    }

    @objc private func confirmDelete() {
        guard let routine = store.routine(routineID), let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Delete “%@”?", routine.name)
        alert.informativeText = L("This deletes the routine and stops its future runs. This can't be undone.")
        alert.addButton(withTitle: L("Delete routine"))
        alert.addButton(withTitle: L("Cancel"))
        alert.alertStyle = .warning
        alert.beginSheetModal(for: window) { [weak self] response in
            guard let self, response == .alertFirstButtonReturn else { return }
            self.store.deleteRoutine(self.routineID)
            self.dismiss(nil)
        }
    }
}
