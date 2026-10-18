import AppKit
import TextchumKit

/// The debug panel, under the editor: what a debug session is doing, the
/// stack of the thread that stopped, the variables of the frame chosen in
/// it, and a console with the program's output and a field to evaluate
/// expressions in that frame. The session is the core's
/// (`textchum_dap`); this draws what it says and sends what is asked.
@MainActor
final class DebugPanel: NSView, NSTableViewDataSource, NSTableViewDelegate, NSOutlineViewDataSource,
    NSOutlineViewDelegate
{
    /// The session on show, and the core that runs it.
    private(set) var sessionID: UInt64?
    weak var core: CoreApp?
    /// The panel's height when shown; zero when not.
    var heightConstraint: NSLayoutConstraint?
    static let shownHeight: CGFloat = 240
    /// A frame was stopped in or chosen: its file and zero-based line.
    var onFrame: ((String, Int) -> Void)?
    /// The program is running again, or the session is over: no line is
    /// the one it is stopped at any more.
    var onResumed: (() -> Void)?

    struct Frame {
        let id: Int
        let name: String
        let path: String?
        let line: Int?
    }

    /// A scope or a variable in the variables list.
    final class Node {
        let name: String
        let value: String
        let type: String?
        let reference: Int
        let isScope: Bool
        /// Nil until asked for, for a variable that holds others.
        var children: [Node]?
        init(name: String, value: String, type: String?, reference: Int, isScope: Bool, children: [Node]? = nil) {
            self.name = name
            self.value = value
            self.type = type
            self.reference = reference
            self.isScope = isScope
            self.children = children
        }
    }

    private(set) var frames: [Frame] = []
    private var scopes: [Node] = []
    private var chosenFrame: Int?
    private var stopped = false
    /// The session failed, and its status says why rather than "ended".
    private var failed = false

    private let title = NSTextField(labelWithString: "")
    private let status = NSTextField(labelWithString: "")
    private lazy var continueButton = button(t("Continue"), #selector(continueClicked(_:)))
    private lazy var stepOverButton = button(t("Step Over"), #selector(stepOverClicked(_:)))
    private lazy var stepIntoButton = button(t("Step Into"), #selector(stepIntoClicked(_:)))
    private lazy var stepOutButton = button(t("Step Out"), #selector(stepOutClicked(_:)))
    private lazy var stopButton = button(t("Stop"), #selector(stopClicked(_:)))
    private lazy var closeButton = button(t("Close"), #selector(closeClicked(_:)))
    private let framesTable = NSTableView()
    private let variablesOutline = NSOutlineView()
    private let console = NSTextView()
    private let evaluateField = NSTextField()

    var isShown: Bool { (heightConstraint?.constant ?? 0) > 0 }
    /// What the smoke test reads back.
    var statusText: String { status.stringValue }
    var frameNames: [String] { frames.map(\.name) }
    var consoleText: String { console.string }
    var variableTexts: [String] {
        var texts: [String] = []
        func walk(_ nodes: [Node]) {
            for node in nodes {
                texts.append(node.isScope ? node.name : "\(node.name) = \(node.value)")
                if let children = node.children, variablesOutline.isItemExpanded(node) { walk(children) }
            }
        }
        walk(scopes)
        return texts
    }

    override init(frame: NSRect) {
        super.init(frame: frame)
        translatesAutoresizingMaskIntoConstraints = false
        let small = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        title.font = NSFont.boldSystemFont(ofSize: NSFont.smallSystemFontSize)
        title.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        status.font = small
        status.textColor = .secondaryLabelColor
        status.lineBreakMode = .byTruncatingTail
        status.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let header = NSStackView(views: [
            title, status, NSView(), continueButton, stepOverButton, stepIntoButton, stepOutButton, stopButton,
            closeButton,
        ])
        header.orientation = .horizontal
        header.spacing = 6
        header.edgeInsets = NSEdgeInsets(top: 4, left: 10, bottom: 4, right: 10)

        let frameColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("frame"))
        framesTable.addTableColumn(frameColumn)
        framesTable.headerView = nil
        framesTable.rowSizeStyle = .small
        framesTable.dataSource = self
        framesTable.delegate = self
        framesTable.setAccessibilityLabel(t("Call Stack"))
        let variableColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("variable"))
        variablesOutline.addTableColumn(variableColumn)
        variablesOutline.outlineTableColumn = variableColumn
        variablesOutline.headerView = nil
        variablesOutline.rowSizeStyle = .small
        variablesOutline.dataSource = self
        variablesOutline.delegate = self
        variablesOutline.setAccessibilityLabel(t("Variables"))
        console.isEditable = false
        console.isSelectable = true
        console.drawsBackground = false
        console.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        console.textContainerInset = NSSize(width: 4, height: 4)
        console.autoresizingMask = [.width]
        evaluateField.placeholderString = t("An expression to evaluate where the program stopped — ⏎")
        evaluateField.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        evaluateField.target = self
        evaluateField.action = #selector(evaluate(_:))

        let split = NSSplitView()
        split.isVertical = true
        split.dividerStyle = .thin
        split.addArrangedSubview(scrolled(framesTable))
        split.addArrangedSubview(scrolled(variablesOutline))
        let consoleSide = NSStackView(views: [scrolled(console), evaluateField])
        consoleSide.orientation = .vertical
        consoleSide.spacing = 2
        split.addArrangedSubview(consoleSide)

        let stack = NSStackView(views: [header, split])
        stack.orientation = .vertical
        stack.spacing = 0
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        let separator = NSBox()
        separator.boxType = .separator
        separator.translatesAutoresizingMaskIntoConstraints = false
        addSubview(separator)
        NSLayoutConstraint.activate([
            separator.topAnchor.constraint(equalTo: topAnchor),
            separator.leadingAnchor.constraint(equalTo: leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: separator.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            header.leadingAnchor.constraint(equalTo: stack.leadingAnchor),
            header.trailingAnchor.constraint(equalTo: stack.trailingAnchor),
            split.leadingAnchor.constraint(equalTo: stack.leadingAnchor),
            split.trailingAnchor.constraint(equalTo: stack.trailingAnchor),
        ])
        clipsToBounds = true
        setSteppable(false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("DebugPanel is built in code") }

    private func button(_ title: String, _ action: Selector) -> NSButton {
        let button = NSButton(title: title, target: self, action: action)
        button.bezelStyle = .inline
        button.font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        return button
    }

    private func scrolled(_ view: NSView) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.documentView = view
        scroll.hasVerticalScroller = true
        scroll.borderType = .noBorder
        scroll.widthAnchor.constraint(greaterThanOrEqualToConstant: 160).isActive = true
        return scroll
    }

    /// The step buttons work only while the program is stopped.
    private func setSteppable(_ steppable: Bool) {
        stopped = steppable
        for button in [continueButton, stepOverButton, stepIntoButton, stepOutButton] {
            button.isEnabled = steppable
        }
        evaluateField.isEnabled = steppable
    }

    /// Shows a session that just started, in place of the one before,
    /// which is ended and let go of.
    func begin(id: UInt64, title text: String) {
        if let previous = sessionID, previous != id {
            core?.debugForget(id: previous)
        }
        sessionID = id
        failed = false
        title.stringValue = text
        status.stringValue = t("starting")
        frames = []
        scopes = []
        chosenFrame = nil
        framesTable.reloadData()
        variablesOutline.reloadData()
        console.string = ""
        stopButton.isEnabled = true
        setSteppable(false)
        heightConstraint?.constant = Self.shownHeight
    }

    /// What the session said.
    func handle(kind: String, json: String) {
        let body = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
        switch kind {
        case "output":
            appendConsole(body["text"] as? String ?? "")
        case "running", "continued":
            status.stringValue = t("running")
            frames = []
            scopes = []
            framesTable.reloadData()
            variablesOutline.reloadData()
            setSteppable(false)
            onResumed?()
        case "breakpoints":
            for point in body["breakpoints"] as? [[String: Any]] ?? [] where point["verified"] as? Bool == false {
                let line = (point["line"] as? Int).map { String($0 + 1) } ?? "?"
                let path = ((body["path"] as? String) ?? "") as NSString
                appendConsole(
                    t("The breakpoint at {}:{} is not set: {}", path.lastPathComponent, line,
                        point["message"] as? String ?? t("the debugger did not say why")) + "\n")
            }
        case "stopped":
            frames = (body["frames"] as? [[String: Any]] ?? []).map {
                Frame(id: $0["id"] as? Int ?? 0, name: $0["name"] as? String ?? "?", path: $0["path"] as? String,
                    line: $0["line"] as? Int)
            }
            let reason = body["description"] as? String ?? body["reason"] as? String ?? ""
            status.stringValue = reason.isEmpty ? t("stopped") : t("stopped: {}", reason)
            setSteppable(true)
            framesTable.reloadData()
            if !frames.isEmpty {
                framesTable.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
                choose(frame: 0)
            }
        case "variables":
            guard body["frame"] as? Int == chosenFrame else { return }
            scopes = (body["scopes"] as? [[String: Any]] ?? []).map { scope in
                Node(name: scope["name"] as? String ?? "", value: "", type: nil, reference: 0, isScope: true,
                    children: Self.nodes(scope["variables"]))
            }
            variablesOutline.reloadData()
            for scope in scopes { variablesOutline.expandItem(scope) }
        case "children":
            guard let reference = body["reference"] as? Int, let node = find(reference: reference, in: scopes)
            else { return }
            node.children = Self.nodes(body["variables"])
            variablesOutline.reloadItem(node, reloadChildren: true)
            variablesOutline.expandItem(node)
        case "evaluated":
            let result = body["result"] as? String ?? ""
            appendConsole("› \(body["expression"] as? String ?? "")\n\(result)\n")
        case "exited":
            appendConsole(t("— exited with {}", String(body["exitCode"] as? Int ?? 0)) + "\n")
        case "waiting":
            status.stringValue = body["message"] as? String ?? ""
            appendConsole((body["message"] as? String ?? "") + "\n")
        case "failed":
            failed = true
            status.stringValue = body["message"] as? String ?? t("failed")
            appendConsole((body["message"] as? String ?? "") + "\n")
        case "ended":
            if !failed {
                status.stringValue = t("ended")
            }
            stopButton.isEnabled = false
            setSteppable(false)
            onResumed?()
        default:
            break
        }
    }

    private static func nodes(_ value: Any?) -> [Node] {
        (value as? [[String: Any]] ?? []).map {
            Node(name: $0["name"] as? String ?? "", value: $0["value"] as? String ?? "", type: $0["type"] as? String,
                reference: $0["reference"] as? Int ?? 0, isScope: false)
        }
    }

    private func find(reference: Int, in nodes: [Node]) -> Node? {
        for node in nodes {
            if !node.isScope, node.reference == reference { return node }
            if let found = find(reference: reference, in: node.children ?? []) { return found }
        }
        return nil
    }

    private func appendConsole(_ text: String) {
        guard !text.isEmpty, let storage = console.textStorage else { return }
        storage.append(
            NSAttributedString(
                string: text,
                attributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular), .foregroundColor: NSColor.labelColor,
                ]))
        console.scrollToEndOfDocument(nil)
    }

    /// A frame of the stack chosen: its variables asked for, its line
    /// shown.
    func choose(frame index: Int) {
        guard frames.indices.contains(index), let sessionID else { return }
        let frame = frames[index]
        chosenFrame = frame.id
        scopes = []
        variablesOutline.reloadData()
        core?.debugSend(id: sessionID, ["command": "variables", "frame": frame.id])
        if let path = frame.path, let line = frame.line {
            onFrame?(path, line)
        }
    }

    /// Continue, Step Over, Step Into, Step Out, Stop: false when there
    /// is no session, or the program is not stopped for a step.
    @discardableResult
    func perform(_ command: String) -> Bool {
        guard let sessionID, let core else { return false }
        if command != "stop", !stopped { return false }
        return core.debugSend(id: sessionID, ["command": command])
    }

    @objc private func continueClicked(_ sender: Any?) { perform("continue") }
    @objc private func stepOverClicked(_ sender: Any?) { perform("next") }
    @objc private func stepIntoClicked(_ sender: Any?) { perform("stepIn") }
    @objc private func stepOutClicked(_ sender: Any?) { perform("stepOut") }
    @objc private func stopClicked(_ sender: Any?) { perform("stop") }
    @objc private func closeClicked(_ sender: Any?) { hide() }

    @objc private func evaluate(_ sender: Any?) {
        let expression = evaluateField.stringValue.trimmingCharacters(in: .whitespaces)
        guard !expression.isEmpty, let sessionID, let frame = chosenFrame else { return }
        core?.debugSend(id: sessionID, ["command": "evaluate", "frame": frame, "expression": expression])
        evaluateField.stringValue = ""
    }

    /// Evaluates `expression` in the chosen frame, for the smoke test.
    func evaluateForTest(_ expression: String) {
        evaluateField.stringValue = expression
        evaluate(nil)
    }

    /// Opens a variable that holds others, as a click on its triangle
    /// would, for the smoke test.
    func expandForTest(named name: String) {
        for row in 0..<variablesOutline.numberOfRows {
            if let node = variablesOutline.item(atRow: row) as? Node, node.name == name {
                variablesOutline.expandItem(node)
                return
            }
        }
    }

    /// Puts the panel away; a session still going keeps going.
    func hide() {
        heightConstraint?.constant = 0
        window?.makeFirstResponder(nil)
    }

    func show() {
        guard sessionID != nil else { return }
        heightConstraint?.constant = Self.shownHeight
    }

    // MARK: Lists

    func numberOfRows(in tableView: NSTableView) -> Int { frames.count }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let frame = frames[row]
        let place = frame.path.map { ($0 as NSString).lastPathComponent + (frame.line.map { ":\($0 + 1)" } ?? "") }
        return label(place.map { "\(frame.name)  \($0)" } ?? frame.name, dim: frame.path == nil)
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        let row = framesTable.selectedRow
        if row >= 0, frames.indices.contains(row), frames[row].id != chosenFrame {
            choose(frame: row)
        }
    }

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let node = item as? Node else { return scopes.count }
        return node.children?.count ?? 0
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        guard let node = item as? Node else { return scopes[index] }
        return node.children?[index] ?? node
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        guard let node = item as? Node else { return false }
        return node.isScope ? !(node.children ?? []).isEmpty : node.reference > 0
    }

    /// A variable opened for the first time asks what it holds; its rows
    /// come when the answer does.
    func outlineView(_ outlineView: NSOutlineView, shouldExpandItem item: Any) -> Bool {
        if let node = item as? Node, !node.isScope, node.children == nil, node.reference > 0, let sessionID {
            core?.debugSend(id: sessionID, ["command": "children", "reference": node.reference])
        }
        return true
    }

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let node = item as? Node else { return nil }
        if node.isScope { return label(node.name, dim: true) }
        let type = node.type.map { "  \($0)" } ?? ""
        return label("\(node.name) = \(node.value)\(type)", dim: false)
    }

    private func label(_ text: String, dim: Bool) -> NSView {
        let field = NSTextField(labelWithString: text)
        field.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        field.textColor = dim ? .secondaryLabelColor : .labelColor
        field.lineBreakMode = .byTruncatingTail
        field.toolTip = text
        return field
    }
}
