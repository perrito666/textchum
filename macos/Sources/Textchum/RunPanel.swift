import AppKit
import TextchumKit

/// The run panel, under the editor: a command's output as it comes, in
/// its colours, with the places it names underlined to open with a
/// click and walked with Next and Previous Output Place. While the
/// command runs, a line can be sent to its input — which is what a
/// command that has gone quiet is often waiting for, and the panel says
/// so. The output is the core's (`textchum_core::output`); this draws it.
///
/// When the output reports tests, they are listed beside it by suite,
/// each with how it went; choosing one shows the line that reported it,
/// and Rerun Failed runs again only the ones that failed.
@MainActor
final class RunPanel: NSView, NSTextViewDelegate, NSTextFieldDelegate, NSOutlineViewDataSource,
    NSOutlineViewDelegate
{
    /// The run on show, and the core that runs it.
    private(set) var runID: UInt64?
    /// How the run in the panel ended: its exit code, nil while it runs
    /// or when it was stopped.
    private(set) var exitCode: Int?
    weak var core: CoreApp?
    /// Opens a place the output names: path, zero-based line and column.
    var onOpenPlace: ((String, Int, Int) -> Void)?
    /// Runs a command line in a folder: Rerun Failed.
    var onRun: ((String, String) -> Void)?
    /// The panel's height when shown; zero when not.
    var heightConstraint: NSLayoutConstraint?
    static let shownHeight: CGFloat = 220

    private(set) var command = ""
    /// Where the run on show was started, which is where it runs again.
    private(set) var cwd: String?
    private let title = NSTextField(labelWithString: "")
    private let status = NSTextField(labelWithString: "")
    private let stopButton = NSButton(title: t("Stop"), target: nil, action: nil)
    private let closeButton = NSButton(title: t("Close"), target: nil, action: nil)
    private let rerunButton = NSButton(title: t("Rerun Failed"), target: nil, action: nil)
    private let summary = NSTextField(labelWithString: "")
    private let split = NSSplitView()
    private let testsScroll = NSScrollView()
    /// Typing filters the tests (3.3); the field and the list go together.
    private let testsFilter = NSSearchField()
    private let testsSide = NSStackView()
    private let testsOutline = TestOutline()
    /// The tests as last listed, and the rows standing for them.
    private(set) var tests: CoreApp.RunTests?
    private var suiteRows: [TestRow] = []
    private let scroll = NSScrollView()
    private let text = NSTextView()
    private let input = NSTextField()
    /// Each output line's length in UTF-16 units, so a changed line can
    /// be found in the text without measuring the whole of it.
    private var lineLengths: [Int] = []
    /// The places each output line names: (start, end, path, line, column).
    private var places: [[(Int, Int, String, Int, Int)]] = []
    /// Where Next Output Place stepped last: an output line and a start.
    private var stepped: (line: Int, character: Int)?

    var isShown: Bool { (heightConstraint?.constant ?? 0) > 0 }
    /// Whether the tests are listed beside the output, for the smoke test.
    var showsTests: Bool { !testsSide.isHidden }
    /// The tests listed, as the filter leaves them, for the smoke test.
    var listedTestNames: [String] { suiteRows.flatMap { $0.children.map(\.name) } }
    var summaryText: String { summary.stringValue }
    /// The command Rerun Failed would run, when there is one.
    var rerunCommand: String? { tests?.rerun }
    /// What the panel shows, for the smoke test.
    var shownText: String { text.string }
    var statusText: String { status.stringValue }

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
        summary.font = small
        summary.lineBreakMode = .byTruncatingTail
        summary.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        for button in [rerunButton, stopButton, closeButton] {
            button.bezelStyle = .inline
            button.font = small
            button.target = self
        }
        stopButton.action = #selector(stop(_:))
        closeButton.action = #selector(close(_:))
        rerunButton.action = #selector(rerunFailed(_:))
        rerunButton.isHidden = true
        let header = NSStackView(views: [title, status, summary, NSView(), rerunButton, stopButton, closeButton])
        header.orientation = .horizontal
        header.spacing = 8
        header.edgeInsets = NSEdgeInsets(top: 4, left: 10, bottom: 4, right: 10)

        text.isEditable = false
        text.isSelectable = true
        text.drawsBackground = false
        text.delegate = self
        text.textContainerInset = NSSize(width: 6, height: 4)
        text.isAutomaticLinkDetectionEnabled = false
        text.linkTextAttributes = [.underlineStyle: NSUnderlineStyle.single.rawValue, .cursor: NSCursor.pointingHand]
        text.autoresizingMask = [.width]
        scroll.documentView = text
        scroll.hasVerticalScroller = true
        scroll.borderType = .noBorder

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("test"))
        testsOutline.addTableColumn(column)
        testsOutline.outlineTableColumn = column
        testsOutline.headerView = nil
        testsOutline.rowSizeStyle = .small
        testsOutline.style = .sourceList
        testsOutline.dataSource = self
        testsOutline.delegate = self
        testsOutline.setAccessibilityLabel(t("Tests"))
        // A double-click is Return: either runs the test alone.
        testsOutline.target = self
        testsOutline.doubleAction = #selector(runChosenTest(_:))
        testsOutline.onReturn = { [weak self] in self?.runChosenTest() }
        testsScroll.documentView = testsOutline
        testsScroll.hasVerticalScroller = true
        testsScroll.borderType = .noBorder
        testsFilter.placeholderString = t("Filter tests")
        testsFilter.font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
        testsFilter.sendsSearchStringImmediately = true
        testsFilter.target = self
        testsFilter.action = #selector(filterChanged(_:))
        testsSide.orientation = .vertical
        testsSide.spacing = 2
        testsSide.addArrangedSubview(testsFilter)
        testsSide.addArrangedSubview(testsScroll)
        testsSide.isHidden = true
        split.isVertical = true
        split.dividerStyle = .thin
        split.addArrangedSubview(testsSide)
        split.addArrangedSubview(scroll)
        split.setHoldingPriority(.defaultHigh, forSubviewAt: 0)
        testsSide.widthAnchor.constraint(greaterThanOrEqualToConstant: 160).isActive = true

        input.placeholderString = t("A line for its input — ⏎ sends it")
        input.font = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        input.delegate = self
        input.target = self
        input.action = #selector(send(_:))

        let stack = NSStackView(views: [header, split, input])
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
            input.leadingAnchor.constraint(equalTo: stack.leadingAnchor, constant: 8),
            input.trailingAnchor.constraint(equalTo: stack.trailingAnchor, constant: -8),
        ])
        clipsToBounds = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("RunPanel is built in code") }

    /// Shows a run that just started in `cwd`, in place of the one
    /// before, which is let go of.
    func begin(id: UInt64, command: String, cwd: String?) {
        if let previous = runID, previous != id {
            core?.runForget(id: previous)
        }
        runID = id
        exitCode = nil
        self.command = command
        self.cwd = cwd
        title.stringValue = command
        tests = nil
        suiteRows = []
        testsOutline.reloadData()
        testsSide.isHidden = true
        testsFilter.stringValue = ""
        summary.stringValue = ""
        rerunButton.isHidden = true
        status.stringValue = t("running")
        text.string = ""
        lineLengths = []
        places = []
        stepped = nil
        stopButton.isEnabled = true
        input.isHidden = false
        input.stringValue = ""
        heightConstraint?.constant = Self.shownHeight
    }

    /// What the core said about the run on show.
    func handle(status news: String) {
        guard let id = runID, let core else { return }
        switch news {
        case "quiet":
            status.stringValue = t("no output for a while — it may be waiting for input")
        default:
            if let update = core.runRead(id: id) {
                apply(update)
                refreshTests()
                if !update.running {
                    exitCode = update.exit
                    status.stringValue = t("finished")
                    stopButton.isEnabled = false
                    input.isHidden = true
                } else if news == "output" {
                    status.stringValue = t("running")
                }
            }
        }
    }

    private func apply(_ update: CoreApp.RunUpdate) {
        guard let storage = text.textStorage else { return }
        let atBottom = scroll.contentView.bounds.maxY >= text.frame.height - 4
        let first = update.reset ? 0 : min(update.first, lineLengths.count)
        let offset = lineLengths.prefix(first).reduce(0, +)
        let replaced = NSRange(location: offset, length: storage.length - offset)
        let rendered = NSMutableAttributedString()
        lineLengths.removeSubrange(first...)
        places.removeSubrange(min(first, places.count)...)
        for (index, line) in update.lines.enumerated() {
            let piece = render(line, outputLine: first + index)
            let isLast = index == update.lines.count - 1
            if !isLast { piece.append(NSAttributedString(string: "\n", attributes: Self.plain)) }
            lineLengths.append(piece.length)
            rendered.append(piece)
        }
        storage.replaceCharacters(in: replaced, with: rendered)
        if atBottom {
            text.scrollToEndOfDocument(nil)
        }
    }

    private static var plain: [NSAttributedString.Key: Any] {
        [.font: NSFont.monospacedSystemFont(ofSize: 11, weight: .regular), .foregroundColor: NSColor.labelColor]
    }

    private func render(_ line: CoreApp.RunUpdate.Line, outputLine: Int) -> NSMutableAttributedString {
        let piece = NSMutableAttributedString(string: line.text, attributes: Self.plain)
        let characters = Array(line.text)
        // Characters to UTF-16 offsets, as the core counts in characters.
        func utf16(_ character: Int) -> Int {
            String(characters.prefix(max(0, min(character, characters.count)))).utf16.count
        }
        for span in line.spans {
            guard span.count == 4, let start = span[0].number, let end = span[1].number else { continue }
            let range = NSRange(location: utf16(start), length: utf16(end) - utf16(start))
            if let hex = span[2].text, let colour = Self.colour(hex) {
                piece.addAttribute(.foregroundColor, value: colour, range: range)
            }
            if span[3].flag {
                piece.addAttribute(.font, value: NSFont.monospacedSystemFont(ofSize: 11, weight: .bold), range: range)
            }
        }
        var named: [(Int, Int, String, Int, Int)] = []
        for (index, place) in line.places.enumerated() {
            guard place.count == 5, let start = place[0].number, let end = place[1].number,
                let path = place[2].text, let line = place[3].number, let column = place[4].number
            else { continue }
            named.append((start, end, path, line, column))
            let range = NSRange(location: utf16(start), length: utf16(end) - utf16(start))
            piece.addAttribute(.link, value: "textchum-place:\(outputLine):\(index)", range: range)
        }
        places.append(named)
        return piece
    }

    private static func colour(_ hex: String) -> NSColor? {
        guard hex.hasPrefix("#"), let value = UInt32(hex.dropFirst(), radix: 16) else { return nil }
        return NSColor(
            srgbRed: CGFloat(value >> 16 & 0xff) / 255, green: CGFloat(value >> 8 & 0xff) / 255,
            blue: CGFloat(value & 0xff) / 255, alpha: 1)
    }

    /// A place clicked in the output opens.
    func textView(_ textView: NSTextView, clickedOnLink link: Any, at charIndex: Int) -> Bool {
        guard let link = link as? String, link.hasPrefix("textchum-place:") else { return false }
        let parts = link.dropFirst("textchum-place:".count).split(separator: ":").compactMap { Int($0) }
        guard parts.count == 2, places.indices.contains(parts[0]), places[parts[0]].indices.contains(parts[1])
        else { return true }
        let place = places[parts[0]][parts[1]]
        stepped = (parts[0], place.0)
        onOpenPlace?(place.2, place.3, place.4)
        return true
    }

    /// Next Output Place (F4) and Previous (⇧F4): the place after or
    /// before the last one gone to, round the end, marked in the output
    /// and opened. False when the output names none.
    @discardableResult
    func step(forward: Bool) -> Bool {
        guard let id = runID, let core,
            let place = core.runNeighbouringPlace(id: id, from: stepped, forward: forward)
        else { return false }
        stepped = (place.outputLine, place.start)
        // The place marked in the output: the core counts characters, the
        // text view UTF-16 units.
        let all = text.string as NSString
        let offset = lineLengths.prefix(place.outputLine).reduce(0, +)
        if lineLengths.indices.contains(place.outputLine), offset <= all.length {
            let lineRange = NSRange(location: offset, length: min(lineLengths[place.outputLine], all.length - offset))
            let characters = Array(all.substring(with: lineRange))
            let before = String(characters.prefix(place.start)).utf16.count
            let length = String(characters.dropFirst(place.start).prefix(place.end - place.start)).utf16.count
            let range = NSRange(location: offset + before, length: length)
            text.setSelectedRange(range)
            text.scrollRangeToVisible(range)
        }
        onOpenPlace?(place.path, place.line, place.column)
        return true
    }

    // MARK: Tests

    /// A suite or a test in the list.
    final class TestRow {
        let name: String
        /// The suite as the core names it, for running a test alone.
        let suite: String
        let test: CoreApp.RunTests.Test?
        let children: [TestRow]
        init(name: String, suite: String, test: CoreApp.RunTests.Test?, children: [TestRow] = []) {
            self.name = name
            self.suite = suite
            self.test = test
            self.children = children
        }
    }

    /// The test list, where Return runs the chosen test as a double-click
    /// does, and never reaches the text.
    final class TestOutline: NSOutlineView {
        var onReturn: (() -> Void)?
        override func keyDown(with event: NSEvent) {
            if event.keyCode == 36 || event.keyCode == 76 {
                onReturn?()
            } else {
                super.keyDown(with: event)
            }
        }
    }

    /// Lists the tests the output reported, when they changed.
    private func refreshTests() {
        guard let id = runID, let core, let found = core.runTests(id: id, query: testsFilter.stringValue),
            found != tests
        else { return }
        tests = found
        suiteRows = found.suites.map { suite in
            TestRow(
                name: suite.name.isEmpty ? t("package not reported yet") : suite.name, suite: suite.name, test: nil,
                children: suite.tests.map { TestRow(name: $0.name, suite: suite.name, test: $0) })
        }
        let reported = found.passed + found.failed + found.skipped + found.running > 0
        if reported && testsSide.isHidden {
            testsSide.isHidden = false
            split.adjustSubviews()
            split.setPosition(min(320, max(200, bounds.width * 0.3)), ofDividerAt: 0)
        }
        testsOutline.reloadData()
        testsOutline.expandItem(nil, expandChildren: true)
        summary.stringValue = found.summary
        rerunButton.isHidden = found.rerun == nil
    }

    @objc private func filterChanged(_ sender: Any?) {
        refreshTests()
    }

    /// Types `query` into the tests' filter, for the smoke test.
    func filterTests(_ query: String) {
        testsFilter.stringValue = query
        refreshTests()
    }

    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        (item as? TestRow)?.children.count ?? suiteRows.count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        (item as? TestRow)?.children[index] ?? suiteRows[index]
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        !((item as? TestRow)?.children.isEmpty ?? true)
    }

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
        guard let row = item as? TestRow else { return nil }
        let identifier = NSUserInterfaceItemIdentifier("testCell")
        let cell = outlineView.makeView(withIdentifier: identifier, owner: nil) as? NSTableCellView ?? {
            let cell = NSTableCellView()
            cell.identifier = identifier
            let image = NSImageView()
            let label = NSTextField(labelWithString: "")
            label.lineBreakMode = .byTruncatingMiddle
            label.font = NSFont.systemFont(ofSize: NSFont.smallSystemFontSize)
            for view in [image, label] as [NSView] {
                view.translatesAutoresizingMaskIntoConstraints = false
                cell.addSubview(view)
            }
            NSLayoutConstraint.activate([
                image.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
                image.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
                image.widthAnchor.constraint(equalToConstant: 14),
                label.leadingAnchor.constraint(equalTo: image.trailingAnchor, constant: 4),
                label.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -2),
                label.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            ])
            cell.imageView = image
            cell.textField = label
            return cell
        }()
        let (symbol, colour) = Self.look(of: row)
        cell.imageView?.image = NSImage(systemSymbolName: symbol, accessibilityDescription: row.test?.outcome)
        cell.imageView?.contentTintColor = colour
        var text = row.name
        if let duration = row.test?.duration { text += "  \(duration)" }
        cell.textField?.stringValue = text
        cell.textField?.toolTip = row.name
        return cell
    }

    /// A test's symbol and colour by how it went; a suite's by its worst.
    private static func look(of row: TestRow) -> (String, NSColor) {
        let outcomes = row.test.map { [$0.outcome] } ?? row.children.compactMap { $0.test?.outcome }
        if outcomes.contains("failed") { return ("xmark.circle.fill", .systemRed) }
        if outcomes.contains("unfinished") { return ("exclamationmark.circle", .systemOrange) }
        if outcomes.contains("running") { return ("clock", .secondaryLabelColor) }
        if !outcomes.isEmpty && outcomes.allSatisfy({ $0 == "skipped" }) {
            return ("minus.circle", .secondaryLabelColor)
        }
        return ("checkmark.circle.fill", .systemGreen)
    }

    /// A test chosen: the output line that reported it, shown and selected.
    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard let row = testsOutline.item(atRow: testsOutline.selectedRow) as? TestRow,
            let line = row.test?.line
        else { return }
        showOutputLine(line)
    }

    /// Selects output line `line` and scrolls it into view.
    func showOutputLine(_ line: Int) {
        guard lineLengths.indices.contains(line) else { return }
        let offset = lineLengths.prefix(line).reduce(0, +)
        let length = lineLengths[line] - (line < lineLengths.count - 1 ? 1 : 0)
        let range = NSRange(location: offset, length: max(0, min(length, text.string.utf16.count - offset)))
        text.setSelectedRange(range)
        text.scrollRangeToVisible(range)
    }

    /// The output line selected, for the smoke test.
    var selectedOutputText: String { (text.string as NSString).substring(with: text.selectedRange()) }

    /// Chooses the test called `name` in the list, as a click would.
    func chooseTest(named name: String) {
        for row in 0..<testsOutline.numberOfRows {
            if (testsOutline.item(atRow: row) as? TestRow)?.test?.name == name {
                testsOutline.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
                return
            }
        }
    }

    /// Rerun Failed: only the tests that failed, from where the run was
    /// started. False when there is nothing to run again.
    @discardableResult
    func rerunFailed() -> Bool {
        guard let rerun = tests?.rerun, let onRun else { return false }
        onRun(rerun, cwd ?? "")
        return true
    }

    @objc private func rerunFailed(_ sender: Any?) {
        rerunFailed()
    }

    /// Return or a double-click on a test: that test alone, from where
    /// the run was started. False when no test is chosen or the run's
    /// command cannot be narrowed to it, which the status says.
    @discardableResult
    func runChosenTest() -> Bool {
        guard let id = runID, let core, let onRun,
            let row = testsOutline.item(atRow: testsOutline.selectedRow) as? TestRow, let test = row.test
        else { return false }
        guard let command = core.runRerunTest(id: id, suite: row.suite, name: test.name) else {
            status.stringValue = t("This run's command cannot be narrowed to one test.")
            return false
        }
        onRun(command, cwd ?? "")
        return true
    }

    @objc private func runChosenTest(_ sender: Any?) {
        runChosenTest()
    }

    @objc private func send(_ sender: Any?) {
        guard let id = runID, !input.stringValue.isEmpty else { return }
        if core?.runWrite(id: id, line: input.stringValue) == true {
            input.stringValue = ""
            status.stringValue = t("running")
        }
    }

    @objc func stop(_ sender: Any?) {
        guard let id = runID else { return }
        core?.runStop(id: id)
    }

    @objc private func close(_ sender: Any?) {
        hide()
    }

    /// Puts the panel away; a run still going keeps going, and the panel
    /// comes back when it is shown again.
    func hide() {
        heightConstraint?.constant = 0
        window?.makeFirstResponder(nil)
    }

    func show() {
        guard runID != nil else { return }
        heightConstraint?.constant = Self.shownHeight
    }
}
