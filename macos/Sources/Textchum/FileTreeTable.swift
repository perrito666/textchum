import AppKit
import Combine
import SwiftUI
import TextchumKit

/// The project tree's rows, as a table whose rows are all one height.
///
/// This used to be a SwiftUI List. A List on macOS is an NSTableView
/// that measures rows as they come into view and estimates the rest,
/// and expanding folders inserts rows in the middle of it: after a few
/// such insertions AppKit's row-height cache calls back into itself
/// while resizing the table — "Application performed a reentrant
/// operation in its NSTableView delegate. This warning will become an
/// assert in the future." Nothing of ours was on that stack; a bare
/// List with a ForEach does it (macOS 26.6), and no row height, list
/// style or transaction changes it. Turning the measuring off on the
/// List's own table made SwiftUI measure every row instead, fifteen
/// times the cost of a first scroll through a monorepo.
///
/// A file tree has nothing to measure: every row is one line. So the
/// table is ours, its row height is the sidebar's, there is no height
/// delegate and therefore no height cache, and its total is known
/// rather than guessed. The model is unchanged — FileTreeState still
/// flattens the expansion set into the visible rows this shows.
struct FileTreeTable: NSViewRepresentable {
    let rows: [VisibleTreeRow]
    let expanded: Set<URL>
    let projectRoot: String
    /// Dirty-by-path for every open file, and the focused file's path,
    /// so a row can say "open", "in front", and "unsaved" at a glance.
    let openFiles: [String: Bool]
    let currentPath: String?
    let highlighted: URL?
    let onToggle: (URL) -> Void
    let onOpenFile: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let table = TreeTableView()
        let column = NSTableColumn(identifier: .init("tree"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.style = .sourceList
        // The sidebar's own row height, which follows the system's
        // sidebar size; fixed for every row, which is the point.
        table.rowSizeStyle = .default
        table.backgroundColor = .clear
        table.selectionHighlightStyle = .none
        table.allowsEmptySelection = true
        table.intercellSpacing = NSSize(width: 0, height: 0)
        table.columnAutoresizingStyle = .uniformColumnAutoresizingStyle
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.onClick = { [weak coordinator = context.coordinator] row in
            coordinator?.activate(row: row)
        }
        table.menuProvider = { [weak coordinator = context.coordinator] row in
            coordinator?.menu(forRow: row)
        }
        table.onAppearanceChange = { [weak table] in table?.reloadData() }
        context.coordinator.table = table
        context.coordinator.followIconNews()
        table.postsFrameChangedNotifications = true
        NotificationCenter.default.addObserver(
            context.coordinator, selector: #selector(Coordinator.tableFrameChanged(_:)),
            name: NSView.frameDidChangeNotification, object: table)

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.automaticallyAdjustsContentInsets = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        // The scroll view is hung a little above the view that holds
        // it, which clips: see hideTopPadding.
        let holder = NSView()
        holder.clipsToBounds = true
        holder.addSubview(scroll)
        let top = scroll.topAnchor.constraint(equalTo: holder.topAnchor)
        context.coordinator.scrollTop = top
        NSLayoutConstraint.activate([
            top,
            scroll.leadingAnchor.constraint(equalTo: holder.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: holder.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: holder.bottomAnchor),
        ])
        return holder
    }

    func updateNSView(_ holder: NSView, context: Context) {
        context.coordinator.show(self)
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        weak var table: NSTableView?
        var scrollTop: NSLayoutConstraint?
        private var tree: FileTreeTable?
        /// A reveal whose row does not exist yet: its folders are
        /// expanded, their listings still being read. It is scrolled to
        /// when it arrives.
        private var owedScroll: URL?

        func show(_ next: FileTreeTable) {
            let previous = tree
            tree = next
            guard let table else { return }
            let sameRows =
                previous.map {
                    $0.rows.count == next.rows.count
                        && zip($0.rows, next.rows).allSatisfy {
                            $0.node.url == $1.node.url && $0.depth == $1.depth
                        }
                } ?? false
            if !sameRows {
                // Every row is the same height, so the rows above the
                // viewport keep their places and the view does not move.
                table.reloadData()
                hideTopPadding(of: table)
            } else if previous?.expanded != next.expanded
                || previous?.openFiles != next.openFiles
                || previous?.currentPath != next.currentPath
                || previous?.highlighted != next.highlighted
                || previous?.projectRoot != next.projectRoot
            {
                // The same lines saying something else: only the ones
                // on screen have anything to redraw.
                table.enumerateAvailableRowViews { rowView, row in
                    guard next.rows.indices.contains(row),
                        let cell = rowView.view(atColumn: 0) as? FileTreeCell
                    else { return }
                    self.configure(cell, row: row)
                }
            }
            if previous?.highlighted != next.highlighted {
                owedScroll = next.highlighted
            }
            if let wanted = owedScroll,
                let row = next.rows.firstIndex(where: { $0.node.url == wanted })
            {
                owedScroll = nil
                // Off this turn: the table has been told its rows and
                // has not laid them out yet.
                DispatchQueue.main.async { [weak table] in
                    guard let table, row < table.numberOfRows else { return }
                    NSAnimationContext.runAnimationGroup { context in
                        context.allowsImplicitAnimation = true
                        table.scrollRowToVisible(row)
                    }
                }
            }
        }

        /// The source-list style pads the top of the table, which would
        /// put a gap between the project's name and its first row. The
        /// scroll view is raised by that much and its holder clips it,
        /// so the padding is never in view — a negative content inset
        /// does the same to the rows, but lets them draw over the name
        /// on their way out. The padding is not guessed at: it is
        /// wherever the first row starts.
        private func hideTopPadding(of table: NSTableView) {
            guard table.numberOfRows > 0 else { return }
            let padding = table.rect(ofRow: 0).minY
            if scrollTop?.constant != -padding { scrollTop?.constant = -padding }
        }

        private var iconNews: AnyCancellable?

        /// A file type's system icon is worked out in the background; a
        /// row drawn before the answer wears its badge until told.
        func followIconNews() {
            iconNews = FileIconNews.shared.$edition.dropFirst().sink { [weak self] _ in
                guard let self, let table = self.table else { return }
                table.enumerateAvailableRowViews { rowView, row in
                    guard let cell = rowView.view(atColumn: 0) as? FileTreeCell else { return }
                    cell.forgetWhatIsShown()
                    self.configure(cell, row: row)
                }
            }
        }

        /// The table has been laid out, which is when its first row is
        /// where the style puts it.
        @objc func tableFrameChanged(_ note: Notification) {
            guard let table else { return }
            hideTopPadding(of: table)
        }

        func numberOfRows(in tableView: NSTableView) -> Int { tree?.rows.count ?? 0 }

        func tableView(
            _ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int
        ) -> NSView? {
            let identifier = NSUserInterfaceItemIdentifier("file-tree-cell")
            let cell =
                tableView.makeView(withIdentifier: identifier, owner: nil) as? FileTreeCell
                ?? {
                    let made = FileTreeCell()
                    made.identifier = identifier
                    return made
                }()
            configure(cell, row: row)
            return cell
        }

        /// The tree marks the file in front itself; a selection would
        /// be a second mark that means nothing.
        func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool { false }

        private func configure(_ cell: FileTreeCell, row: Int) {
            guard let tree, tree.rows.indices.contains(row) else { return }
            let line = tree.rows[row]
            let path = line.node.url.path
            let light =
                table?.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua]) != .darkAqua
            cell.show(
                FileTreeCell.Line(
                    name: line.node.name,
                    depth: line.depth,
                    isDirectory: line.node.isDirectory,
                    packagePath: line.node.isPackage ? path : nil,
                    isExpanded: tree.expanded.contains(line.node.url),
                    // nil: not open. Otherwise whether it has unsaved
                    // changes.
                    openAndDirty: line.node.isDirectory ? nil : tree.openFiles[path],
                    isInFront: path == tree.currentPath,
                    isHighlighted: tree.highlighted == line.node.url,
                    light: light))
        }

        /// What a click on a row does: a folder opens or closes, a file
        /// opens.
        func activate(row: Int) {
            guard let tree, tree.rows.indices.contains(row) else { return }
            let node = tree.rows[row].node
            if node.isPackage {
                // An application is not something to open in an editor,
                // and a click should not take it apart. Shut, it stays
                // shut — its menu opens it; open, a click shuts it.
                if tree.expanded.contains(node.url) { tree.onToggle(node.url) }
            } else if node.isDirectory {
                tree.onToggle(node.url)
            } else {
                tree.onOpenFile(node.url.path)
            }
        }

        func menu(forRow row: Int) -> NSMenu? {
            guard let tree, tree.rows.indices.contains(row) else { return nil }
            let node = tree.rows[row].node
            let menu = NSMenu()
            if node.isPackage {
                let shown = tree.expanded.contains(node.url)
                let toggle = tree.onToggle
                menu.addItem(
                    ClosureMenuItem(
                        title: shown ? t("Hide Package Contents") : t("Show Package Contents")
                    ) { toggle(node.url) })
                menu.addItem(.separator())
            }
            for entry in PathActions.menuEntries(
                path: node.url.path, projectRoot: tree.projectRoot,
                isDirectory: node.isDirectory)
            {
                menu.addItem(entry.map { ClosureMenuItem(title: $0.title, run: $0.run) } ?? .separator())
            }
            return menu
        }
    }
}

/// The table itself: says which row was clicked, asks for a row's menu,
/// and says when light turned to dark, since the file icons come in
/// both.
final class TreeTableView: NSTableView {
    var menuProvider: ((Int) -> NSMenu?)?
    var onClick: ((Int) -> Void)?
    var onAppearanceChange: (() -> Void)?

    /// A click is the mouse going down and coming up on the same row.
    /// The table's own action is no use here: with selection refused it
    /// has nothing to report, and sends nothing.
    private var pressedRow: Int?

    override func mouseDown(with event: NSEvent) {
        let pressed = row(at: convert(event.locationInWindow, from: nil))
        pressedRow = pressed >= 0 ? pressed : nil
        super.mouseDown(with: event)
        // When the table tracks the mouse itself it returns only once
        // the button is up, having taken that event; otherwise the
        // release arrives below like any other.
        if let release = NSApp.currentEvent, release.type == .leftMouseUp {
            finishClick(at: release)
        }
    }

    override func mouseUp(with event: NSEvent) {
        super.mouseUp(with: event)
        finishClick(at: event)
    }

    private func finishClick(at release: NSEvent) {
        guard let pressed = pressedRow else { return }
        pressedRow = nil
        if row(at: convert(release.locationInWindow, from: nil)) == pressed {
            onClick?(pressed)
        }
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let row = self.row(at: convert(event.locationInWindow, from: nil))
        return row >= 0 ? menuProvider?(row) : nil
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        onAppearanceChange?()
    }
}

/// A menu item that runs a closure, for menus built where there is no
/// responder to aim a selector at.
final class ClosureMenuItem: NSMenuItem {
    private let run: () -> Void

    init(title: String, run: @escaping () -> Void) {
        self.run = run
        super.init(title: title, action: #selector(fire), keyEquivalent: "")
        target = self
    }

    @available(*, unavailable)
    required init(coder: NSCoder) { fatalError("ClosureMenuItem is built in code") }

    @objc private func fire() { run() }
}

/// One line of the tree: indentation, a chevron on folders, the icon,
/// the name, and on the right what the editor knows about the file —
/// unsaved, open, in front.
final class FileTreeCell: NSTableCellView {
    struct Line: Equatable {
        let name: String
        let depth: Int
        let isDirectory: Bool
        /// Set for a package, which wears its own Finder icon.
        let packagePath: String?
        let isExpanded: Bool
        let openAndDirty: Bool?
        let isInFront: Bool
        let isHighlighted: Bool
        let light: Bool
    }

    private let highlight = NSView()
    private let chevron = NSImageView()
    private let icon = NSImageView()
    private let name = NSTextField(labelWithString: "")
    private let dirtyDot = NSImageView()
    private let openDot = NSImageView()
    private var indent: NSLayoutConstraint!
    private var iconLeading: NSLayoutConstraint!
    private var nameLeading: NSLayoutConstraint!
    /// How far the name may run: to the edge, or to the marks when the
    /// row wears any. Hidden marks hold no room.
    private var nameLimit: NSLayoutConstraint!
    private static let inset: CGFloat = 7
    private var shown: Line?
    /// Makes the next `show` draw from scratch: the line is the same
    /// and what it resolves to — its icon — is not.
    func forgetWhatIsShown() { shown = nil }

    /// The name on the row, for the tests to read.
    var shownName: String? { shown?.name }

    init() {
        super.init(frame: .zero)
        highlight.wantsLayer = true
        highlight.layer?.cornerRadius = 4
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        // The cell's text field follows the sidebar's size with the row.
        textField = name
        for view in [highlight, chevron, icon, name, dirtyDot, openDot] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        for image in [chevron, icon, dirtyDot, openDot] {
            image.imageScaling = .scaleNone
            image.setContentHuggingPriority(.required, for: .horizontal)
        }
        dirtyDot.image = Self.symbol("circle.fill", size: 7, weight: .regular)
        dirtyDot.contentTintColor = .labelColor
        indent = chevron.leadingAnchor.constraint(equalTo: leadingAnchor, constant: Self.inset)
        iconLeading = icon.leadingAnchor.constraint(equalTo: chevron.trailingAnchor, constant: 4)
        nameLeading = name.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 4)
        nameLimit = name.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -3)
        NSLayoutConstraint.activate([
            highlight.leadingAnchor.constraint(equalTo: leadingAnchor),
            highlight.trailingAnchor.constraint(equalTo: trailingAnchor),
            highlight.topAnchor.constraint(equalTo: topAnchor, constant: 2),
            highlight.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -2),
            indent,
            chevron.widthAnchor.constraint(equalToConstant: 10),
            chevron.centerYAnchor.constraint(equalTo: centerYAnchor),
            iconLeading,
            icon.widthAnchor.constraint(equalToConstant: 17),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            nameLeading,
            nameLimit,
            name.centerYAnchor.constraint(equalTo: centerYAnchor),
            dirtyDot.trailingAnchor.constraint(equalTo: openDot.leadingAnchor, constant: -4),
            dirtyDot.centerYAnchor.constraint(equalTo: centerYAnchor),
            openDot.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -3),
            openDot.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("FileTreeCell is built in code") }

    func show(_ line: Line) {
        // A cell scrolled back in, or a row that says what it said: the
        // images are the dear part, and they have not changed.
        guard line != shown else { return }
        shown = line
        name.stringValue = line.name
        indent.constant = Self.inset + CGFloat(line.depth) * 12
        if let packagePath = line.packagePath {
            // As Finder shows it: one item, its own icon. The chevron
            // appears only while its contents have been asked for.
            chevron.isHidden = !line.isExpanded
            chevron.image = line.isExpanded
                ? Self.symbol("chevron.down", size: 9, weight: .semibold) : nil
            chevron.contentTintColor = .secondaryLabelColor
            iconLeading.constant = line.isExpanded ? 4 : -10
            nameLeading.constant = 4
            icon.image = Self.packageIcon(at: packagePath)
            icon.contentTintColor = nil
        } else if line.isDirectory {
            chevron.isHidden = false
            chevron.image = Self.symbol(
                line.isExpanded ? "chevron.down" : "chevron.right", size: 9, weight: .semibold)
            chevron.contentTintColor = .secondaryLabelColor
            iconLeading.constant = 4
            nameLeading.constant = 6
            icon.image = Self.symbol("folder", size: 13, weight: .regular)
            icon.contentTintColor = .secondaryLabelColor
        } else {
            // A file's icon stands where a folder's chevron does.
            chevron.isHidden = true
            chevron.image = nil
            iconLeading.constant = -10
            nameLeading.constant = 4
            let fileIcon = Self.fileIcon(for: line.name, light: line.light)
            icon.image = fileIcon.image
            icon.contentTintColor = fileIcon.isTemplate ? .secondaryLabelColor : nil
        }
        nameLimit.constant = line.openAndDirty.map { $0 ? -30 : -17 } ?? -3
        if let dirty = line.openAndDirty {
            dirtyDot.isHidden = !dirty
            openDot.isHidden = false
            openDot.image = Self.symbol(
                line.isInFront ? "circle.fill" : "circle", size: 6, weight: .regular)
            openDot.contentTintColor = line.isInFront ? .controlAccentColor : .tertiaryLabelColor
        } else {
            dirtyDot.isHidden = true
            openDot.isHidden = true
        }
        highlight.layer?.backgroundColor =
            line.isHighlighted
            ? NSColor.controlAccentColor.withAlphaComponent(0.22).cgColor : NSColor.clear.cgColor
    }

    /// The accent colour is resolved into the layer, so it has to be
    /// resolved again when the appearance it was resolved for changes.
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        if let line = shown {
            forgetWhatIsShown()
            show(line)
        }
    }

    // MARK: Images

    private static var symbols: [String: NSImage] = [:]

    private static func symbol(_ name: String, size: CGFloat, weight: NSFont.Weight) -> NSImage? {
        let key = "\(name)|\(size)|\(weight.rawValue)"
        if let cached = symbols[key] { return cached }
        let image = NSImage(systemSymbolName: name, accessibilityDescription: nil)?
            .withSymbolConfiguration(.init(pointSize: size, weight: weight))
        image?.isTemplate = true
        symbols[key] = image
        return image
    }

    private static var badges: [String: NSImage] = [:]
    private static var packageIcons: [String: NSImage] = [:]

    /// A package's own icon, as Finder draws it. The image comes back
    /// at once and reads its artwork when drawn, so this costs a row
    /// nothing worth putting off.
    private static func packageIcon(at path: String) -> NSImage {
        if let cached = packageIcons[path] { return cached }
        let icon = NSWorkspace.shared.icon(forFile: path)
        icon.size = NSSize(width: 16, height: 16)
        packageIcons[path] = icon
        return icon
    }

    /// The icon for a file, in the order FileTypeIcon settles it: the
    /// icon pack, the system's icon where it says something, the
    /// language badge, the plain document.
    private static func fileIcon(for filename: String, light: Bool) -> (image: NSImage?, isTemplate: Bool) {
        if let packed = CoreIcons.icon(forFilename: filename, language: nil, light: light) {
            if packed.size != NSSize(width: 16, height: 16) {
                packed.size = NSSize(width: 16, height: 16)
            }
            return (packed, false)
        }
        if let system = SystemFileIcon.icon(forFilename: filename) { return (system, false) }
        guard let badge = LanguageBadge.badge(for: filename) else {
            return (symbol("doc.text", size: 12, weight: .regular), true)
        }
        let key = "\(badge.label)|\(badge.darkText)"
        if let cached = badges[key] { return (cached, false) }
        let size = NSSize(width: 17, height: 13)
        let image = NSImage(size: size, flipped: false) { rect in
            NSColor(badge.color).setFill()
            NSBezierPath(roundedRect: rect, xRadius: 3, yRadius: 3).fill()
            let font = NSFont.systemFont(ofSize: 7.5, weight: .bold)
            let rounded =
                font.fontDescriptor.withDesign(.rounded).flatMap { NSFont(descriptor: $0, size: 7.5) }
                ?? font
            let text = NSAttributedString(
                string: badge.label,
                attributes: [
                    .font: rounded,
                    .foregroundColor: badge.darkText
                        ? NSColor.black.withAlphaComponent(0.72) : NSColor.white,
                ])
            let bounds = text.size()
            text.draw(
                at: NSPoint(
                    x: (rect.width - bounds.width) / 2, y: (rect.height - bounds.height) / 2))
            return true
        }
        badges[key] = image
        return (image, false)
    }
}
