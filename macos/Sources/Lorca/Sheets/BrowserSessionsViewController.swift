import AppKit

/// The CLI owns the profile, assigned Runner, and exclusive input gate. This
/// sheet makes the user's transfer of control explicit, including from a paired Mac.
final class BrowserSessionsViewController: SheetViewController {
    private let store = AppStore.shared
    private let bot: Bot
    private let chatID: Chat.ID
    private let runner: Device
    private let accountField = NSTextField(string: L("Default"))
    private let profileField = NSTextField(string: L("Browser"))
    private let picker = NSPopUpButton()
    private let detail = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let status = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let capabilityNote = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let idLabel = Build.label("", font: .monospacedSystemFont(ofSize: 10, weight: .regular), color: .tertiaryLabelColor, lines: 0)
    private lazy var createButton = NSButton(title: L("Create Profile"), target: self, action: #selector(createProfile))
    private lazy var openButton = NSButton(title: L("Open Browser"), target: self, action: #selector(openBrowser))
    private lazy var controlButton = NSButton(title: L("Take Over"), target: self, action: #selector(changeControl))
    private lazy var stopButton = NSButton(title: L("Stop Browser"), target: self, action: #selector(stopBrowser))
    private lazy var screenshotButton = NSButton(title: L("Attach Screenshot"), target: self, action: #selector(screenshot))
    private var sessions: [BrowserSession] = []
    private var capabilities: BrowserCapabilities?
    private var selectedID: String?
    private var busy = false
    private var poll: Task<Void, Never>?
    private var refreshing = false

    init(bot: Bot, chatID: Chat.ID, runner: Device) {
        self.bot = bot
        self.chatID = chatID
        self.runner = runner
        super.init(title: L("Browser Sessions"), subtitle: L("%@ owns these profiles on %@. Sign in in the visible browser, then explicitly return control to the bot.", bot.name, runner.name), width: 570)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let fields = NSGridView(views: [
            [Build.label(L("Account"), font: Theme.Font.caption), accountField],
            [Build.label(L("Profile"), font: Theme.Font.caption), profileField],
        ])
        fields.translatesAutoresizingMaskIntoConstraints = false
        fields.column(at: 0).width = 70
        fields.rowSpacing = 8
        contentStack.addArrangedSubview(fields)
        contentStack.addArrangedSubview(createButton)
        contentStack.addArrangedSubview(Build.label(L("Each profile has separate browser data. For stronger separation, assign the bot to a dedicated Runner."), font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0))
        picker.target = self
        picker.action = #selector(selectionChanged)
        contentStack.addArrangedSubview(picker)
        contentStack.addArrangedSubview(detail)
        idLabel.isSelectable = true
        contentStack.addArrangedSubview(idLabel)
        let controls = Build.stack([openButton, controlButton, stopButton], orientation: .horizontal, spacing: 8)
        contentStack.addArrangedSubview(controls)
        contentStack.addArrangedSubview(screenshotButton)
        contentStack.addArrangedSubview(capabilityNote)
        contentStack.addArrangedSubview(status)
        for button in [createButton, openButton, controlButton, stopButton, screenshotButton] {
            button.bezelStyle = .rounded
        }
        for child in [fields, picker, detail, idLabel, capabilityNote, status] {
            child.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        setButtons(confirm: L("Done"), cancel: nil)
        render()
    }

    override func viewWillAppear() {
        super.viewWillAppear()
        poll = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refresh()
                do { try await Task.sleep(nanoseconds: 2_000_000_000) } catch { return }
            }
        }
    }

    override func viewWillDisappear() {
        super.viewWillDisappear()
        poll?.cancel()
        poll = nil
    }

    private var selected: BrowserSession? { sessions.first { $0.id == selectedID } }

    private func refresh() async {
        guard !refreshing else { return }
        refreshing = true
        defer { refreshing = false }
        do {
            let data = try await store.client.request("browser.sessions", ["bot_id": bot.id])
            let list = try JSONDecoder().decode(BrowserSessionList.self, from: data)
            sessions = list.sessions
            capabilities = list.capabilities
            if !sessions.contains(where: { $0.id == selectedID }) {
                selectedID = sessions.first(where: \.selected)?.id ?? sessions.first?.id
            }
            render()
        } catch {
            status.stringValue = error.localizedDescription
        }
    }

    private func render() {
        picker.removeAllItems()
        picker.addItems(withTitles: sessions.map { $0.title + ($0.selected ? " · " + L("Selected") : "") })
        if let index = sessions.firstIndex(where: { $0.id == selectedID }) { picker.selectItem(at: index) }
        picker.isEnabled = !sessions.isEmpty && !busy
        createButton.isEnabled = !busy && store.isConnected
        accountField.isEnabled = !busy
        profileField.isEnabled = !busy
        let session = selected
        idLabel.stringValue = session?.id ?? ""
        let state: String
        switch session?.state {
        case "bot": state = L("Bot control")
        case "taking_over": state = L("Waiting for current input to finish…")
        case "human": state = L("Human control on %@", runner.name)
        default: state = L("Stopped")
        }
        detail.stringValue = session == nil ? L("Create a profile to start a persistent browser session.") : state
        openButton.isEnabled = capabilities?.visibleOpen == true && session != nil && !busy && session?.isTakingOver != true
        controlButton.title = session?.isHuman == true ? L("Return to Bot") : (capabilities?.localInput == true ? L("Take Over") : L("Pause on Runner"))
        controlButton.isEnabled = session != nil && session?.isStopped == false && session?.isTakingOver == false && !busy
        // Stop stays available while Open or Take Over is waiting on the Runner.
        stopButton.isEnabled = session != nil && (session?.isStopped == false || busy)
        screenshotButton.isEnabled = session != nil && session?.isStopped == false && session?.isTakingOver == false && !busy
        capabilityNote.stringValue = capabilities?.localInput == true
            ? L("Use the browser on this Runner while you have control. Bot tool calls wait until Return to Bot. Screenshots attach encrypted evidence to this chat.")
            : L("This Device can pause, return control, stop, and attach screenshots. Sign-in and interactive input happen on %@; live remote viewing and input are unavailable.", runner.name)
        fitSheetToContent()
    }

    @objc private func selectionChanged() {
        guard sessions.indices.contains(picker.indexOfSelectedItem) else { return }
        selectedID = sessions[picker.indexOfSelectedItem].id
        render()
    }

    private func perform(_ method: String, additional: [String: Any] = [:], stopping: Bool = false) {
        var params: [String: Any] = ["bot_id": bot.id, "chat_id": chatID]
        if let session = selected {
            params["session_id"] = session.id
            params["revision"] = session.revision
        }
        params.merge(additional) { _, new in new }
        if !stopping { busy = true }
        status.stringValue = method == "browser.takeover" ? L("Waiting for current input to finish…") : L("Working…")
        render()
        Task { [weak self] in
            guard let self else { return }
            defer { if !stopping { self.busy = false }; self.render() }
            do {
                let data = try await self.store.client.request(method, params)
                if method == "browser.create", let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any], let session = result["session"] as? [String: Any], let id = session["id"] as? String {
                    self.selectedID = id
                }
                self.status.stringValue = method == "browser.screenshot" ? L("Screenshot attached in chat.") : ""
                await self.refresh()
            } catch {
                self.status.stringValue = error.localizedDescription
                await self.refresh()
            }
        }
    }

    @objc private func createProfile() { perform("browser.create", additional: ["account": accountField.stringValue, "profile": profileField.stringValue]) }
    @objc private func openBrowser() { perform("browser.open") }
    @objc private func changeControl() { perform(selected?.isHuman == true ? "browser.resume" : "browser.takeover") }
    @objc private func stopBrowser() { perform("browser.stop", stopping: true) }
    @objc private func screenshot() { perform("browser.screenshot") }
}
