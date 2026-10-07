import AppKit
import QuickLookUI

/// Native file actions use a CLI-provided, private local copy with its original extension.
@MainActor
enum OutputFileActions {
    private static let previewer = Previewer()

    static func preview(_ url: URL) {
        previewer.url = url
        guard let panel = QLPreviewPanel.shared() else { return }
        panel.dataSource = previewer
        panel.reloadData()
        panel.makeKeyAndOrderFront(nil)
    }

    static func save(_ url: URL, name: String, window: NSWindow?) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = URL(fileURLWithPath: name).lastPathComponent
        let finish: (NSApplication.ModalResponse) -> Void = { response in
            guard response == .OK, let target = panel.url, target != url else { return }
            do {
                try Data(contentsOf: url).write(to: target, options: .atomic)
            } catch {
                NSAlert(error: error).runModal()
            }
        }
        if let window { panel.beginSheetModal(for: window, completionHandler: finish) }
        else { panel.begin(completionHandler: finish) }
    }

    private final class Previewer: NSObject, QLPreviewPanelDataSource {
        var url: URL?
        func numberOfPreviewItems(in panel: QLPreviewPanel!) -> Int { url == nil ? 0 : 1 }
        func previewPanel(_ panel: QLPreviewPanel!, previewItemAt index: Int) -> QLPreviewItem! { url as NSURL? }
    }
}

/// A result surface independent of the transcript's currently loaded page. All synced versions
/// appear newest first, with an explicit control for older history still held by the relay.
final class OutputsViewController: NSViewController {
    let chatID: Chat.ID
    private let store = AppStore.shared
    private let rows = OutputStack()
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let earlier = NSButton(title: L("Load earlier outputs"), target: nil, action: nil)
    private var loading = false
    private var reloadPending = false

    init(chatID: Chat.ID) {
        self.chatID = chatID
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 560, height: 620))
        preferredContentSize = view.frame.size
        let title = Build.label(L("Outputs"), font: .systemFont(ofSize: 20, weight: .semibold))
        let close = NSButton(title: L("Done"), target: self, action: #selector(done))
        close.keyEquivalent = "\r"
        let refresh = NSButton(title: L("Refresh"), target: self, action: #selector(refreshOutputs))
        earlier.target = self
        earlier.action = #selector(loadEarlier)
        earlier.isHidden = true
        let header = NSStackView(views: [title, NSView(), refresh, close])
        header.orientation = .horizontal
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        rows.orientation = .vertical
        rows.alignment = .leading
        rows.spacing = 18
        rows.translatesAutoresizingMaskIntoConstraints = false
        scroll.documentView = rows
        let stack = NSStackView(views: [header, status, scroll, earlier])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 20),
            stack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -20),
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            header.widthAnchor.constraint(equalTo: stack.widthAnchor),
            scroll.widthAnchor.constraint(equalTo: stack.widthAnchor),
            rows.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
        ])
        store.observe(self) { [weak self] event in
            guard let self else { return }
            switch event {
            case let .messageAdded(id, _), let .messageChanged(id, _), let .messageRemoved(id, _), let .olderMessagesLoaded(id):
                if id == chatID { reload() }
            case .snapshotReplaced, .connectionChanged: reload()
            default: break
            }
        }
        reload()
    }

    private func reload() {
        guard !loading else { reloadPending = true; return }
        loading = true
        status.stringValue = L("Loading outputs…")
        Task { [weak self] in
            guard let self else { return }
            defer {
                loading = false
                if reloadPending { reloadPending = false; reload() }
            }
            do {
                let result = try await store.outputMessages(in: chatID)
                for row in rows.arrangedSubviews { rows.removeArrangedSubview(row); row.removeFromSuperview() }
                for message in result.messages.reversed() {
                    let row = OutputRow(message: message, store: store)
                    rows.addArrangedSubview(row)
                    row.widthAnchor.constraint(equalTo: rows.widthAnchor, constant: -12).isActive = true
                }
                let count = result.messages.count
                status.stringValue = count == 0 ? L("No outputs yet.") : (count == 1 ? L("%d output version", count) : L("%d output versions", count))
                earlier.isHidden = !result.hasMore
            } catch {
                status.stringValue = L("Outputs unavailable: %@", error.localizedDescription)
            }
        }
    }

    @objc private func done() { dismiss(nil) }
    @objc private func refreshOutputs() { reload() }
    @objc private func loadEarlier() { store.loadOlderMessages(in: chatID) }

    /// NSClipView positions a short flipped document at the top of its viewport.
    private final class OutputStack: NSStackView {
        override var isFlipped: Bool { true }
    }
}

final class OutputRow: NSStackView {
    private var url: URL?
    private var filename = ""
    private var retryAction: (() -> Void)?

    init(message: Message, store: AppStore) {
        super.init(frame: .zero)
        orientation = .vertical
        alignment = .leading
        spacing = 6
        guard let output = message.output else { return }
        filename = output.name
        let title = Build.label(L("%@ · v%d", output.name, output.version), font: .systemFont(ofSize: 14, weight: .semibold))
        title.lineBreakMode = .byWordWrapping
        title.maximumNumberOfLines = 0
        addArrangedSubview(title)
        title.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor).isActive = true
        let producer = store.bot(output.botId)?.name ?? output.botId
        let provenance = Build.label(L("Produced by %@ · %@", producer, Format.time(message.createdAt)), font: Theme.Font.caption, color: .secondaryLabelColor)
        addArrangedSubview(provenance)
        if let taskID = output.taskId { addText(L("Task: %@", taskID), color: .secondaryLabelColor) }
        if let evidence = output.evidence {
            addText("\(evidence.title) · \(evidence.statusText): \(evidence.summary)", color: evidence.status == "failed" ? .systemRed : .labelColor)
            if let command = evidence.command { addText(L("Command: %@", command), color: .secondaryLabelColor) }
            if let code = evidence.exitCode { addText(L("Exit code: %d", code), color: .secondaryLabelColor) }
        }
        if let document = output.documentURL {
            url = document
            addText(document.host ?? document.absoluteString, color: .secondaryLabelColor)
            addArrangedSubview(NSButton(title: L("Open document"), target: self, action: #selector(open)))
        } else if let attachment = message.attachments.first {
            url = store.localURL(for: attachment, in: output.chatId, messageID: message.id)
            let error = store.attachmentError(for: attachment)
            retryAction = { store.retryAttachment(attachment, in: output.chatId, messageID: message.id) }
            let tile = AttachmentTile()
            tile.configure(attachment, url: url, onUserBubble: false, error: error, onRetry: retryAction)
            let size = AttachmentLayout.frames(for: [attachment], maxWidth: 420).size
            tile.widthAnchor.constraint(equalToConstant: size.width).isActive = true
            tile.heightAnchor.constraint(equalToConstant: size.height).isActive = true
            addArrangedSubview(tile)
            if let error {
                addText(L("File unavailable: %@", error), color: .secondaryLabelColor)
                addArrangedSubview(NSButton(title: L("Retry"), target: self, action: #selector(retry)))
            } else {
                let actions = NSStackView()
                actions.orientation = .horizontal
                for (title, action) in [(L("Preview"), #selector(preview)), (L("Open"), #selector(open)), (L("Save As…"), #selector(save))] {
                    let button = NSButton(title: title, target: self, action: action)
                    button.isEnabled = url != nil
                    actions.addArrangedSubview(button)
                }
                addArrangedSubview(actions)
            }
        } else {
            addText(L("File unavailable"), color: .secondaryLabelColor)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func addText(_ text: String, color: NSColor) {
        let label = Build.label(text, font: Theme.Font.caption, color: color)
        label.lineBreakMode = .byWordWrapping
        label.maximumNumberOfLines = 0
        addArrangedSubview(label)
        label.widthAnchor.constraint(lessThanOrEqualTo: widthAnchor).isActive = true
    }

    @objc private func preview() { if let url { OutputFileActions.preview(url) } }
    @objc private func open() { if let url { NSWorkspace.shared.open(url) } }
    @objc private func save() { if let url { OutputFileActions.save(url, name: filename, window: window) } }
    @objc private func retry() { retryAction?() }
}
