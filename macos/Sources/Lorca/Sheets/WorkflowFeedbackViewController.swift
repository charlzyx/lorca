import AppKit

/// Explicit feedback and immutable revision diffs. Every operation goes through the local CLI.
final class WorkflowFeedbackViewController: SheetViewController {
    private let store = AppStore.shared
    private let bot: Bot
    private let chatID: Chat.ID
    private let column = Build.stack([], spacing: 12)
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private var fetching = false
    var onOpenOrigin: ((Chat.ID, Message.ID) -> Void)?

    init(bot: Bot, chatID: Chat.ID) {
        self.bot = bot
        self.chatID = chatID
        super.init(title: L("Workflow feedback"), subtitle: L("Review specific improvements to %@'s routines and skills. Every change shows its evidence and diff before you accept it.", bot.name), width: 700)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        column.alignment = .leading
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(column)
        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.documentView = document
        contentStack.addArrangedSubview(status)
        contentStack.addArrangedSubview(scroll)
        NSLayoutConstraint.activate([
            status.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 480),
            document.widthAnchor.constraint(equalTo: scroll.widthAnchor),
            column.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            column.topAnchor.constraint(equalTo: document.topAnchor),
            column.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])
        setButtons(confirm: L("Done"), cancel: nil)
        refresh()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            guard let self else { return }
            if case let .workflowFeedbackChanged(id) = event, id == self.bot.id { self.refresh() }
        }
    }

    private func refresh() {
        guard !fetching else { return }
        fetching = true
        status.stringValue = L("Loading…")
        Task { [weak self] in
            guard let self else { return }
            defer { self.fetching = false }
            do {
                let result = try await self.store.workflowFeedback(botID: self.bot.id)
                self.render(result)
                self.status.stringValue = ""
            } catch { self.status.stringValue = error.localizedDescription }
        }
    }
    private func request(_ method: String, _ params: [String: Any] = [:]) {
        guard !fetching else { return }
        fetching = true
        status.stringValue = L("Working…")
        // Disable the current controls while a decision is in flight. A retry retains its diff hash.
        for button in column.subviews.flatMap({ $0.subviews }).compactMap({ $0 as? NSButton }) { button.isEnabled = false }
        Task { [weak self] in
            guard let self else { return }
            do {
                _ = try await self.store.workflowFeedback(botID: self.bot.id, method: method, params: params)
                self.fetching = false
                self.refresh()
            } catch {
                self.fetching = false
                self.status.stringValue = error.localizedDescription
                for button in self.column.subviews.flatMap({ $0.subviews }).compactMap({ $0 as? NSButton }) { button.isEnabled = true }
            }
        }
    }
    private func add(_ view: NSView) {
        column.addArrangedSubview(view)
        view.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -8).isActive = true
    }
    private func label(_ text: String, heading: Bool = false) -> NSTextField {
        Build.label(text, font: heading ? .systemFont(ofSize: 13, weight: .semibold) : Theme.Font.caption, lines: 0)
    }
    private func render(_ data: [String: Any]) {
        for view in column.arrangedSubviews { column.removeArrangedSubview(view); view.removeFromSuperview() }
        let settings = data["settings"] as? [String: Any] ?? [:]
        let interval = settings["review_every_secs"] as? Int
        let periodic = NSPopUpButton()
        periodic.addItems(withTitles: [L("Off"), L("Daily"), L("Weekly")])
        periodic.selectItem(at: interval == nil ? 0 : (interval == 86400 ? 1 : 2))
        periodic.target = self
        periodic.action = #selector(periodicChanged(_:))
        add(Build.stack([label(L("Periodic review"), heading: true), periodic], orientation: .horizontal, spacing: 10))
        add(label(L("Reviews stay quiet when there is no useful proposal. Ignored alerts are neutral; silence does not imply a preference.")))
        let excludedChats = settings["excluded_chats"] as? [String] ?? []
        let exclude = FeedbackActionButton(L("Exclude this chat")) { [weak self] in
            guard let self else { return }
            self.request("feedback.exclude", ["chat_id": self.chatID])
        }
        exclude.isEnabled = !excludedChats.contains(chatID)
        if !exclude.isEnabled { exclude.title = L("This chat is excluded") }
        add(Build.stack([
            FeedbackActionButton(L("Review now")) { [weak self] in self?.request("feedback.review") },
            FeedbackActionButton(L("Refresh")) { [weak self] in self?.refresh() }, exclude,
        ], orientation: .horizontal, spacing: 10))
        add(label(L("Proposed improvements"), heading: true))
        let proposals = (data["proposals"] as? [[String: Any]] ?? []).filter { $0["state"] as? String == "pending" }
        if proposals.isEmpty { add(label(L("No improvements waiting for review."))) }
        for proposal in proposals {
            guard let id = proposal["id"] as? String, let diffHash = proposal["diff_hash"] as? String else { continue }
            add(label(targetName(proposal["target"] as? [String: Any] ?? [:]), heading: true))
            add(label(proposal["explanation"] as? String ?? ""))
            let origins = proposal["origins"] as? [[String: Any]] ?? []
            for origin in origins { add(originRow(origin)) }
            add(Self.textBox(proposal["diff"] as? String ?? "", height: 170))
            add(Build.stack([
                FeedbackActionButton(L("Accept revision")) { [weak self] in self?.request("feedback.accept", ["id": id, "diff_hash": diffHash]) },
                FeedbackActionButton(L("Reject")) { [weak self] in self?.request("feedback.reject", ["id": id, "diff_hash": diffHash]) },
                FeedbackActionButton(L("Exclude this workflow")) { [weak self] in
                    guard let target = proposal["target"] else { return }
                    self?.request("feedback.exclude", ["target": target])
                },
            ], orientation: .horizontal, spacing: 10))
        }
        add(label(L("Feedback examples"), heading: true))
        let feedback = data["feedback"] as? [[String: Any]] ?? []
        if feedback.isEmpty { add(label(L("Use Record workflow feedback… on a message to record acceptance, rejection, edits, or an explicit request."))) }
        for item in feedback.suffix(20).reversed() {
            let excluded = item["excluded"] as? Bool == true
            let kind = feedbackKindTitle(item["kind"] as? String ?? "explicit")
            add(label(excluded ? L("%@ · excluded", kind) : kind, heading: true))
            if !excluded {
                add(label((item["note"] as? String ?? "") + "\n" + (item["example"] as? String ?? "")))
                if let before = item["before"] as? String, let after = item["after"] as? String {
                    add(Self.textBox(L("Original: %@\nEdited: %@", before, after), height: 100))
                }
            }
            let source = originRow(item["origin"] as? [String: Any] ?? [:])
            if let id = item["id"] as? String, !excluded {
                source.addArrangedSubview(FeedbackActionButton(L("Exclude")) { [weak self] in self?.request("feedback.exclude", ["id": id]) })
            }
            add(source)
        }
        add(label(L("Revision history"), heading: true))
        for revision in (data["revisions"] as? [[String: Any]] ?? []).suffix(10).reversed() {
            let version = revision["version"] as? Int ?? 0
            let state = revision["state"] as? String ?? ""
            add(label(L("Version %d · %@", version, state), heading: true))
            if revision["can_rollback"] as? Bool == true, let id = revision["id"] as? String, let hash = revision["current_hash"] as? String {
                add(Self.textBox(revision["rollback_diff"] as? String ?? "", height: 120))
                add(FeedbackActionButton(L("Roll back this revision")) { [weak self] in self?.request("feedback.rollback", ["id": id, "expected_hash": hash]) })
            }
        }
    }
    private func originRow(_ origin: [String: Any]) -> NSStackView {
        let chat = origin["chat_id"] as? String ?? ""
        let message = origin["message_id"] as? String ?? ""
        let button = FeedbackActionButton(L("Open originating work")) { [weak self] in
            guard let self else { return }
            self.dismiss(nil)
            self.onOpenOrigin?(chat, message)
        }
        button.toolTip = "lorca://chat/\(chat)/message/\(message)"
        return Build.stack([button], orientation: .horizontal, spacing: 10)
    }
    private func targetName(_ target: [String: Any]) -> String {
        if target["kind"] as? String == "plugin_skill" { return L("Skill · %@ / %@", target["plugin_id"] as? String ?? "", target["name"] as? String ?? "") }
        if target["kind"] as? String == "playbook" { return L("Playbook · %@", target["id"] as? String ?? "") }
        let id = target["id"] as? String ?? ""
        return L("Routine · %@", store.routines(for: bot.id).first { $0.id == id }?.name ?? id)
    }
    @objc private func periodicChanged(_ sender: NSPopUpButton) {
        let interval: Any = sender.indexOfSelectedItem == 0 ? NSNull() : (sender.indexOfSelectedItem == 1 ? 86400 : 604800)
        request("feedback.settings", ["review_every_secs": interval])
    }
    static func textBox(_ string: String, height: CGFloat, editable: Bool = false) -> NSScrollView {
        let text = NSTextView()
        text.isEditable = editable
        text.isRichText = false
        text.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        text.textColor = .labelColor
        text.textContainerInset = NSSize(width: 8, height: 8)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        text.string = string
        let scroll = NSScrollView()
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.heightAnchor.constraint(equalToConstant: height).isActive = true
        return scroll
    }
}

final class RecordWorkflowFeedbackViewController: SheetViewController {
    private let store = AppStore.shared
    private let botID: Bot.ID
    private let chatID: Chat.ID
    private let message: Message
    private let kind = NSPopUpButton()
    private let target = NSPopUpButton()
    private let note = NSTextField(wrappingLabelWithString: "")
    private var targets: [[String: Any]] = []
    private var edited: NSTextView!
    private let excluded = NSButton(checkboxWithTitle: L("Exclude this material from feedback processing"), target: nil, action: nil)
    private let errorLabel = Build.label("", font: Theme.Font.caption, color: .systemRed, lines: 0)
    private let kinds = ["accepted", "rejected", "edited", "explicit", "ignored_alert"]
    init(botID: Bot.ID, chatID: Chat.ID, message: Message) {
        self.botID = botID; self.chatID = chatID; self.message = message
        super.init(title: L("Record workflow feedback"), subtitle: L("Record your decision or correction with a link to this work. Ignored alerts stay neutral."), width: 580)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        kind.addItems(withTitles: kinds.map(feedbackKindTitle))
        target.addItem(withTitle: L("This work"))
        note.isEditable = true; note.isSelectable = true; note.isBezeled = true; note.drawsBackground = true
        note.placeholderString = L("Your feedback or explanation")
        let edit = WorkflowFeedbackViewController.textBox(message.text, height: 160, editable: true)
        edited = edit.documentView as? NSTextView
        for row in [kind, target, note, Build.label(L("For user edits, put the corrected draft below:"), font: Theme.Font.caption), edit, excluded, errorLabel] as [NSView] {
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        setButtons(confirm: L("Record"))
        Task { [weak self] in
            guard let self, let data = try? await store.workflowFeedback(botID: botID) else { return }
            targets = data["targets"] as? [[String: Any]] ?? []
            target.addItems(withTitles: targets.map { $0["name"] as? String ?? "" })
        }
    }
    override func confirmTapped() {
        var feedback: [String: Any] = ["kind": kinds[kind.indexOfSelectedItem], "origin": ["chat_id": chatID, "message_id": message.id], "note": note.stringValue, "excluded": excluded.state == .on]
        if target.indexOfSelectedItem > 0 { feedback["target"] = targets[target.indexOfSelectedItem - 1]["target"] }
        if kinds[kind.indexOfSelectedItem] == "edited" { feedback["before"] = message.text; feedback["after"] = edited.string }
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                _ = try await store.workflowFeedback(botID: botID, method: "feedback.record", params: ["feedback": feedback])
                dismiss(nil)
            } catch { errorLabel.stringValue = error.localizedDescription; confirmButton.isEnabled = true }
        }
    }
}

private final class FeedbackActionButton: NSButton {
    private let perform: () -> Void
    init(_ title: String, perform: @escaping () -> Void) {
        self.perform = perform
        super.init(frame: .zero)
        self.title = title; bezelStyle = .rounded; target = self; action = #selector(tapped)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    @objc private func tapped() { perform() }
}
private func feedbackKindTitle(_ kind: String) -> String {
    switch kind {
    case "accepted": L("Accepted")
    case "rejected": L("Rejected")
    case "edited": L("User edited")
    case "routine_failure": L("Routine failure")
    case "ignored_alert": L("Ignored alert · neutral")
    default: L("Explicit feedback")
    }
}
