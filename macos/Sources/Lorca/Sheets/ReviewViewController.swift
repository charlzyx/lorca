import AppKit

/// A saved proposal is edited separately from approving it. The sheet keeps its displayed
/// version through sync changes, so an approval can never refer to text the user has not seen.
final class ReviewViewController: SheetViewController {
    private let store = AppStore.shared
    private var item: ReviewItem
    private let account = NSTextField()
    private let resource = NSTextField()
    private let rationale = NSTextField()
    private let editor = NSTextView()
    private let status = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let save = NSButton()
    private let approve = NSButton()
    private let reject = NSButton()
    private let cancel = NSButton()
    private let reload = NSButton()
    private var busy = false

    init(item: ReviewItem) {
        self.item = item
        super.init(title: L("Review item"), subtitle: L("Approve the saved version to resume this proposal on its Runner."), width: 600)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        for (label, field) in [(L("Account"), account), (L("Resource"), resource), (L("Rationale"), rationale)] {
            let row = Build.stack([Build.label(label, font: .systemFont(ofSize: 12)), field], orientation: .horizontal, spacing: 12)
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
            field.setContentHuggingPriority(.defaultLow, for: .horizontal)
            field.setAccessibilityLabel(label)
        }
        let bot = store.bot(item.botId)?.name ?? item.botId
        let runner = store.device(item.runnerId)?.name ?? item.runnerId
        let source = Build.label(L("%@ on %@", bot, runner), font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
        contentStack.addArrangedSubview(source)
        if let tool = item.payload.tool {
            contentStack.addArrangedSubview(Build.label("\(item.payload.pluginId ?? "") / \(item.payload.serverName ?? "") / \(tool)", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0))
        }
        if !item.preconditions.files.isEmpty {
            contentStack.addArrangedSubview(Build.label(L("Guarded files: %@", item.preconditions.files.map(\.path).joined(separator: ", ")), font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0))
        }

        editor.isRichText = false
        editor.setAccessibilityLabel(item.payload.kind == "draft" ? L("Draft") : L("Proposed call arguments"))
        editor.font = item.payload.kind == "draft" ? .systemFont(ofSize: 13) : .monospacedSystemFont(ofSize: 12, weight: .regular)
        editor.textColor = .labelColor
        editor.textContainerInset = NSSize(width: 8, height: 8)
        editor.isVerticallyResizable = true
        editor.autoresizingMask = [.width]
        editor.textContainer?.widthTracksTextView = true
        let scroll = NSScrollView()
        scroll.documentView = editor
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        scroll.translatesAutoresizingMaskIntoConstraints = false
        contentStack.addArrangedSubview(scroll)
        contentStack.addArrangedSubview(status)
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor), scroll.heightAnchor.constraint(equalToConstant: 220),
            status.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])
        let buttons: [(NSButton, String, Selector)] = [
            (save, L("Save Changes"), #selector(saveChanges)), (approve, L("Approve"), #selector(approveItem)),
            (reject, L("Reject"), #selector(rejectItem)), (cancel, L("Cancel Item"), #selector(cancelItem)), (reload, L("Reload"), #selector(reloadItem)),
        ]
        for (button, title, action) in buttons {
            button.title = title
            button.bezelStyle = .rounded
            button.target = self
            button.action = action
        }
        contentStack.addArrangedSubview(Build.stack(buttons.map { $0.0 }, orientation: .horizontal, spacing: 8))
        setButtons(confirm: L("Done"), cancel: nil)
        display(item)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            guard let self else { return }
            switch event {
            case .reviewsChanged, .snapshotReplaced:
                if let current = self.store.review(self.item.id), current.revision != self.item.revision {
                    self.status.stringValue = L("This review changed. Reload it before deciding.")
                    self.approve.isEnabled = false
                }
            default: break
            }
        }
    }

    private func display(_ item: ReviewItem) {
        self.item = item
        account.stringValue = item.target.account
        resource.stringValue = item.target.resource
        rationale.stringValue = item.rationale
        editor.string = item.payload.editorText
        status.stringValue = L("Version %d · %@", Int(item.version), item.stateText) + (item.outcome.map { "\n" + $0.summary } ?? "")
        account.isEditable = item.isEditable
        resource.isEditable = item.isEditable
        rationale.isEditable = item.isEditable
        editor.isEditable = item.isEditable
        setBusy(false)
    }

    private func setBusy(_ value: Bool) {
        busy = value
        account.isEditable = !value && item.isEditable
        resource.isEditable = !value && item.isEditable
        rationale.isEditable = !value && item.isEditable
        editor.isEditable = !value && item.isEditable
        for button in [save, approve, reject, cancel] { button.isEnabled = !value && item.isEditable }
        reload.isEnabled = !value
    }

    private func act(_ action: String, fields: [String: Any] = [:]) {
        guard !busy else { return }
        let displayed = item
        setBusy(true)
        Task { [weak self] in
            guard let self else { return }
            do { self.display(try await self.store.changeReview(displayed, action: action, fields: fields)) }
            catch {
                self.setBusy(false)
                self.status.stringValue = error.localizedDescription
            }
        }
    }

    @objc private func saveChanges() {
        do {
            let payload = try item.payload.parameters(editedText: editor.string)
            act("edit", fields: ["payload": payload, "target": ["account": account.stringValue, "resource": resource.stringValue], "rationale": rationale.stringValue])
        } catch { status.stringValue = error.localizedDescription }
    }

    @objc private func approveItem() {
        guard editor.string == item.payload.editorText, account.stringValue == item.target.account,
            resource.stringValue == item.target.resource, rationale.stringValue == item.rationale else {
            status.stringValue = L("Save your changes, then review and approve the new version.")
            return
        }
        act("approve")
    }
    @objc private func rejectItem() { act("reject") }
    @objc private func cancelItem() { act("cancel") }
    @objc private func reloadItem() {
        guard !busy else { return }
        setBusy(true)
        Task { [weak self] in
            guard let self else { return }
            do { self.display(try await self.store.refreshReview(self.item.id)) }
            catch { self.setBusy(false); self.status.stringValue = error.localizedDescription }
        }
    }
}
