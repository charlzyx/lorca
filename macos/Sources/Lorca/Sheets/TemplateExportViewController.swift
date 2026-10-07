import AppKit
import UniformTypeIdentifiers

final class TemplateExportViewController: SheetViewController {
    private let store = AppStore.shared
    private let bot: Bot
    private let choices = Build.stack([], spacing: 6)
    private let previewText = TemplatePreviewText(height: 235)
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let reviewed = NSButton(checkboxWithTitle: L("I reviewed the selected content for personal information."), target: nil, action: nil)
    private var boxes: [String: [(id: String, box: NSButton)]] = [:]
    private var profileBox: NSButton?
    private var preview: TemplatePreview?
    private var busy = false
    private var generation = 0

    init(bot: Bot) {
        self.bot = bot
        super.init(title: L("Export Bot Template"),
            subtitle: L("Choose reusable content from %@. Saving creates a private file; share it only with people you choose.", bot.name), width: 640)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let list = TemplateChoiceScroll(stack: choices, height: 220)
        for row in [list, previewText, status, reviewed] as [NSView] {
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        reviewed.target = self
        reviewed.action = #selector(reviewChanged)
        reviewed.isEnabled = false
        setButtons(confirm: L("Preview Contents"))
        confirmButton.isEnabled = false
        status.stringValue = L("Loading reusable content…")
        Task { [weak self] in
            guard let self else { return }
            do {
                let json = try await self.store.templateReply("templates.contents", ["bot_id": self.bot.id])
                self.showContents(TemplateContents(json: json))
            } catch { self.showError(error) }
        }
    }

    private func showContents(_ contents: TemplateContents) {
        let profile = NSButton(checkboxWithTitle: L("Profile and instructions: %@", contents.profileName), target: self, action: #selector(selectionChanged))
        profile.state = .off
        choices.addArrangedSubview(profile)
        profileBox = profile
        addItems(L("Reusable skills"), key: "skill_ids", items: contents.skills)
        addItems(L("Selected memories"), key: "memory_ids", items: contents.memories)
        addItems(L("Routines"), key: "routine_ids", items: contents.routines)
        addItems(L("Integration requirements"), key: "requirement_ids", items: contents.requirements)
        status.stringValue = contents.notes.joined(separator: "\n")
        selectionChanged()
    }

    private func addItems(_ title: String, key: String, items: [TemplateContents.Item]) {
        let label = Build.label(title, font: .systemFont(ofSize: 12, weight: .semibold))
        choices.addArrangedSubview(label)
        boxes[key] = items.map { item in
            let caption = String(item.title.replacingOccurrences(of: "\n", with: " ").prefix(110))
            let box = NSButton(checkboxWithTitle: caption, target: self, action: #selector(selectionChanged))
            box.state = .off
            box.toolTip = item.title
            box.setAccessibilityLabel(title + ": " + caption)
            choices.addArrangedSubview(box)
            return (item.id, box)
        }
        if items.isEmpty { choices.addArrangedSubview(Build.label(L("None available"), font: Theme.Font.caption, color: .secondaryLabelColor)) }
    }

    private var selection: [String: Any] {
        var value: [String: Any] = ["profile": profileBox?.state == .on]
        for (key, rows) in boxes { value[key] = rows.filter { $0.box.state == .on }.map(\.id) }
        return value
    }

    private var hasSelection: Bool { profileBox?.state == .on || boxes.values.flatMap { $0 }.contains { $0.box.state == .on } }

    @objc private func selectionChanged() {
        generation += 1
        preview = nil
        reviewed.state = .off
        reviewed.isEnabled = false
        previewText.text = L("Select content, then preview the complete private file before saving.")
        confirmButton.title = L("Preview Contents")
        confirmButton.isEnabled = hasSelection && !busy
    }

    @objc private func reviewChanged() { confirmButton.isEnabled = preview != nil && reviewed.state == .on && !busy }

    override func confirmTapped() {
        if preview != nil { saveFile(); return }
        busy = true
        confirmButton.isEnabled = false
        let selected = selection
        let current = generation
        Task { [weak self] in
            guard let self else { return }
            do {
                let json = try await self.store.templateReply("templates.export.preview", ["bot_id": self.bot.id, "selection": selected])
                guard current == self.generation else {
                    self.busy = false
                    self.confirmButton.isEnabled = self.hasSelection
                    return
                }
                let preview = TemplatePreview(json: json)
                self.preview = preview
                self.previewText.text = preview.text
                self.status.stringValue = L("Review every selected instruction, memory, resource, and script. Credential-like text is redacted; personal content can remain.")
                self.reviewed.isEnabled = true
                self.confirmButton.title = L("Save Private File…")
            } catch { self.showError(error) }
            self.busy = false
            self.confirmButton.isEnabled = self.preview == nil ? self.hasSelection : self.reviewed.state == .on
        }
    }

    private func saveFile() {
        guard let window = view.window, let preview, reviewed.state == .on else { return }
        let panel = NSSavePanel()
        panel.title = L("Save Private Template")
        panel.allowedContentTypes = [UTType(filenameExtension: "lorca-template") ?? .json]
        panel.nameFieldStringValue = "\(bot.name).lorca-template"
        let selected = selection
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let self, let url = panel.url else { return }
            self.busy = true
            self.confirmButton.isEnabled = false
            Task { [weak self] in
                guard let self else { return }
                do {
                    _ = try await self.store.templateReply("templates.export", ["bot_id": self.bot.id, "selection": selected,
                        "path": url.path, "expected_digest": preview.digest, "reviewed": true,
                        "overwrite": FileManager.default.fileExists(atPath: url.path)])
                    self.dismiss(nil)
                } catch {
                    self.selectionChanged()
                    self.showError(error)
                }
                self.busy = false
                self.confirmButton.isEnabled = self.preview == nil ? self.hasSelection : self.reviewed.state == .on
            }
        }
    }

    private func showError(_ error: Error) {
        status.stringValue = error.localizedDescription
        status.textColor = .systemRed
    }
}

/// Read-only plain text makes all selected content inspectable and copyable, including long
/// memories and scripts, with a fixed sheet height and native text accessibility.
final class TemplatePreviewText: NSScrollView {
    private let textView = NSTextView()
    var text: String { get { textView.string } set { textView.string = newValue } }

    init(height: CGFloat) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        hasVerticalScroller = true
        borderType = .bezelBorder
        textView.isRichText = false
        textView.isEditable = false
        textView.isSelectable = true
        textView.font = .systemFont(ofSize: 12)
        textView.textColor = .labelColor
        textView.textContainerInset = NSSize(width: 8, height: 8)
        textView.isVerticallyResizable = true
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.setAccessibilityLabel(L("Template contents preview"))
        documentView = textView
        heightAnchor.constraint(equalToConstant: height).isActive = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

final class TemplateChoiceScroll: NSScrollView {
    init(stack: NSStackView, height: CGFloat) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        hasVerticalScroller = true
        drawsBackground = false
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        stack.alignment = .leading
        document.addSubview(stack)
        documentView = document
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: height),
            document.widthAnchor.constraint(equalTo: widthAnchor, constant: -16),
            stack.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            stack.topAnchor.constraint(equalTo: document.topAnchor),
            stack.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}
