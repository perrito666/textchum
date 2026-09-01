import AppKit
import SwiftUI
import TextchumKit

extension Notification.Name {
    /// Posted whenever the set of open documents, or any document's path,
    /// dirty state, or title changes — the sidebar rebuilds from it.
    static let textchumDocumentsChanged = Notification.Name("textchumDocumentsChanged")
    /// Posted when a window's sidebar divider moves, carrying the new
    /// width. Every other window follows, so the navigator is one
    /// width across the application rather than per window.
    static let textchumSidebarWidthChanged =
        Notification.Name("textchumSidebarWidthChanged")
}

/// One open document as the sidebar sees it.
struct SidebarDocument: Identifiable, Hashable {
    /// Identity of the owning window controller.
    let id: ObjectIdentifier
    let title: String
    let path: String?
    let isDirty: Bool
    /// What the row shows: the title, or the project-relative path while
    /// the full-path toggle is on. Part of the row data on purpose — the
    /// list diffs by value, so a display that changed must change the
    /// value or stale rows survive the toggle.
    var display: String = ""
}

/// Open documents that share a project root.
struct SidebarProjectGroup: Identifiable, Hashable {
    /// The project root path, or nil for the loose-files group.
    let root: String?
    let documents: [SidebarDocument]

    var id: String { root ?? "«loose»" }
    var name: String {
        guard let root else { return "Other" }
        return (root as NSString).lastPathComponent
    }
}

/// Per-window sidebar state: the buffer list shows the documents of this
/// window's tab group only, so separate windows keep separate worlds; the
/// folder tree tracks the window's own document's project.
@MainActor
final class SidebarModel: ObservableObject {
    @Published var groups: [SidebarProjectGroup] = []
    /// Off until the launch has restored what it is going to restore:
    /// an empty list before that says "Loading…", after it just says
    /// nothing is open.
    @Published var settled = false

    /// While on, buffer rows show paths from the project root instead of
    /// names. Session-only by design — a quick look, not a mode, so it is
    /// never persisted.
    @Published var showFullPaths = false {
        didSet { recompute() }
    }

    private var entries: [(document: SidebarDocument, projectRoot: String?)] = []

    /// Rebuilds the grouped buffer list. `entries` pairs each document
    /// with its (cached) project root.
    func rebuild(entries: [(document: SidebarDocument, projectRoot: String?)]) {
        self.entries = entries
        recompute()
    }

    private func recompute() {
        var byRoot: [String?: [SidebarDocument]] = [:]
        for entry in entries {
            var document = entry.document
            if showFullPaths, let path = document.path {
                document.display = PathActions.relativePath(
                    path, projectRoot: entry.projectRoot)
            } else {
                document.display = document.title
            }
            byRoot[entry.projectRoot, default: []].append(document)
        }
        groups = byRoot
            .map { SidebarProjectGroup(root: $0.key, documents: $0.value) }
            .sorted {
                // Named projects alphabetically, loose files last.
                switch ($0.root, $1.root) {
                case (nil, _): return false
                case (_, nil): return true
                default: return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
                }
            }
    }
}

/// Shared file-explorer state: which folders are expanded. One instance
/// serves every window, so the tree looks identical across the tabs of a
/// group (and everywhere else the same project shows).
@MainActor
final class FileTreeState: ObservableObject {
    @Published var expanded: Set<URL> = []
    /// How to resolve a root's hidden-name globs, so a reveal can
    /// prefetch listings without waiting for the rows to render one
    /// level per runloop turn.
    var globsProvider: (String) -> [String] = { _ in [".*"] }
    /// Directory listings the tree renders from, read off the main
    /// thread when a folder is first needed. A folder that is not
    /// expanded is never read — no listing, no stat, nothing.
    @Published private(set) var listings: [URL: [FileNode]] = [:]
    private var pendingListings: Set<URL> = []

    /// The cached listing, or nil while the first read is in flight —
    /// which it starts. Rendering from a cache is what keeps a render
    /// from touching the disk.
    func children(of url: URL, globs: [String]) -> [FileNode]? {
        if let cached = listings[url] { return cached }
        requestListing(of: url, globs: globs)
        return nil
    }

    /// Whether a folder's listing has arrived — the difference between
    /// "empty" and "still reading", which the tree says out loud.
    func hasListing(of url: URL) -> Bool { listings[url] != nil }

    private func requestListing(of url: URL, globs: [String]) {
        guard listings[url] == nil, !pendingListings.contains(url) else { return }
        pendingListings.insert(url)
        DispatchQueue.global(qos: .userInitiated).async {
            let children = FileNode.read(directory: url, globs: globs)
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    self.pendingListings.remove(url)
                    self.listings[url] = children
                }
            }
        }
    }

    /// The tree flattened to what the expansion set shows: one row per
    /// visible node, its depth carried for indentation. An outline that
    /// scales is a plain list underneath — recursion is for building
    /// the array, never for the view tree.
    func visibleRows(root: URL, globs: [String]) -> [VisibleTreeRow] {
        var rows: [VisibleTreeRow] = []
        func walk(_ url: URL, depth: Int) {
            guard let children = children(of: url, globs: globs) else { return }
            for child in children {
                rows.append(VisibleTreeRow(node: child, depth: depth))
                if child.isDirectory, expanded.contains(child.url) {
                    walk(child.url, depth: depth + 1)
                }
            }
        }
        walk(root, depth: 0)
        return rows
    }

    /// Forgets every listing, so what shows next reflects the disk;
    /// called when the app comes back to the front, where files may
    /// have changed underneath it. The expanded folders re-read on
    /// their next render.
    func refreshListings() {
        guard !listings.isEmpty else { return }
        listings = [:]
    }
    /// The file last revealed in the tree, briefly emphasized.
    @Published var highlighted: URL?

    /// Expands every ancestor of `path` under `root` and highlights the
    /// file — Reveal in Tree, and the follow-the-file behavior.
    /// Directory URLs are built directory-shaped: the tree's nodes come
    /// from directory enumeration, whose URLs carry the trailing slash,
    /// and URL equality cares.
    ///
    /// Nothing is published unless something changed, and the change
    /// waits for the next turn of the runloop. This is called from
    /// `windowDidBecomeKey`, which AppKit can send in the middle of a
    /// display cycle: setting state a SwiftUI view is being laid out
    /// from is what AttributeGraph aborts the process over. A file
    /// nested a few folders deep — a Hugo post under content/posts —
    /// is the case where the expansion set really does change.
    func reveal(path: String, under root: String) {
        let rootURL = treeKey(root, isDirectory: true)
        let file = treeKey(path, isDirectory: false)
        var ancestors: Set<URL> = []
        var ancestor = file.deletingLastPathComponent()
        while ancestor.path.hasPrefix(rootURL.path), ancestor.path != "/" {
            ancestors.insert(treeKey(ancestor.path, isDirectory: true))
            if ancestor.path == rootURL.path { break }
            ancestor.deleteLastPathComponent()
        }
        // All the ancestors' listings at once: rendered one at a time,
        // each level waits a turn for the one above it, which is the
        // pause a deep reveal used to show.
        let globs = globsProvider(root)
        for ancestor in ancestors {
            requestListing(of: ancestor, globs: globs)
        }
        guard highlighted != file || !ancestors.isSubset(of: expanded) else { return }
        DispatchQueue.main.async { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                if self.highlighted != file { self.highlighted = file }
                if !ancestors.isSubset(of: self.expanded) {
                    self.expanded.formUnion(ancestors)
                }
            }
        }
    }

    /// Fraction of the sidebar's height given to the buffer list.
    /// Shared like the expansion set, so every tab shows the same split
    /// instead of each remembering its own; persisted with the session.
    @Published var splitFraction: Double = 0.45
    /// Called when a divider drag ends, so the session can be saved
    /// once, not per drag tick.
    var onSplitCommitted: (() -> Void)?

}

/// The one URL form every tree key uses. Directory enumeration hands
/// back `/private/tmp/...` while paths arrive as `/tmp/...` (and
/// `resolvingSymlinksInPath` deliberately leaves `/tmp` alone), so
/// every node, expansion entry, and highlight goes through
/// `standardizingPath` — equal paths, equal URLs, or the disclosure
/// bindings quietly never match.
func treeKey(_ path: String, isDirectory: Bool) -> URL {
    URL(fileURLWithPath: (path as NSString).standardizingPath, isDirectory: isDirectory)
}

/// One visible line of the flattened tree.
struct VisibleTreeRow: Identifiable {
    let node: FileNode
    let depth: Int
    var id: URL { node.url }
}

/// One row of the project tree; the tree is a flat list of these, and
/// expansion just changes which rows exist.
struct FileTreeRow: View {
    let node: FileNode
    let depth: Int
    /// Whether this directory row is expanded — plain data, so the row
    /// re-renders only when its own line changes.
    let isExpanded: Bool
    let projectRoot: String?
    /// Dirty-by-path for every open file, and the focused file's path,
    /// so a row can say "open", "in front", and "unsaved" at a glance.
    var openFiles: [String: Bool] = [:]
    var currentPath: String?
    var highlighted: URL?
    let onToggle: (URL) -> Void
    let onOpenFile: (String) -> Void

    var body: some View {
        label
            .contentShape(Rectangle())
            .onTapGesture {
                if node.isDirectory {
                    onToggle(node.url)
                } else {
                    onOpenFile(node.url.path)
                }
            }
    }

    private var label: some View {
        HStack(spacing: 4) {
            Spacer(minLength: 0).frame(width: CGFloat(depth) * 12)
            if node.isDirectory {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(isExpanded ? 90 : 0))
                    .frame(width: 10)
                Image(systemName: "folder")
                    .foregroundStyle(.secondary)
            } else {
                FileTypeIcon(filename: node.name)
            }
            Text(node.name)
            Spacer(minLength: 0)
            // The file in front wears a filled dot, other open files a
            // hollow one, and unsaved changes the same dot the Open
            // Files list marks them with.
            if !node.isDirectory, let dirty = openFiles[node.url.path] {
                if dirty {
                    Image(systemName: "circle.fill")
                        .font(.system(size: 7))
                        .foregroundStyle(.primary)
                }
                Image(systemName: node.url.path == currentPath ? "circle.fill" : "circle")
                    .font(.system(size: 6))
                    .foregroundStyle(
                        node.url.path == currentPath
                            ? AnyShapeStyle(Color.accentColor)
                            : AnyShapeStyle(.tertiary))
            }
        }
        .padding(.horizontal, 3)
        .background(
            RoundedRectangle(cornerRadius: 4)
                .fill(
                    highlighted == node.url
                        ? Color.accentColor.opacity(0.22) : Color.clear))
        .contextMenu {
            PathCopyMenu(
                path: node.url.path, projectRoot: projectRoot,
                isDirectory: node.isDirectory)
        }
        .id(node.url)
    }
}

/// A file-system node of the project tree. Children are read lazily from
/// disk on expansion; hidden files are skipped.
struct FileNode: Identifiable, Hashable {
    let url: URL
    let isDirectory: Bool
    /// The project's effective hidden-name globs, passed down the tree.
    let hiddenGlobs: [String]

    var id: URL { url }
    var name: String { url.lastPathComponent }

    /// The sorted visible entries of one directory — names the
    /// configuration hides (dotfiles by default; per-project globs on
    /// top) never appear. Called by the tree state's loader, off the
    /// main thread; rendering reads the cache it fills.
    static func read(directory url: URL, globs hiddenGlobs: [String]) -> [FileNode] {
        let entries = (try? FileManager.default.contentsOfDirectory(
            at: url,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: []
        )) ?? []
        return entries
            .filter { !CoreWorkspace.isHidden(name: $0.lastPathComponent, globs: hiddenGlobs) }
            .map { url in
                let isDirectory =
                    (try? url.resourceValues(forKeys: [.isDirectoryKey]))?.isDirectory
                    ?? false
                return FileNode(
                    url: treeKey(url.path, isDirectory: isDirectory),
                    isDirectory: isDirectory,
                    hiddenGlobs: hiddenGlobs
                )
            }
            .sorted {
                // Directories first, then case-insensitive by name.
                if $0.isDirectory != $1.isDirectory { return $0.isDirectory }
                return $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
            }
    }
}

/// The navigation drawer: open buffers grouped by project on top, the
/// folder tree of this window's project below.
struct SidebarView: View {
    @ObservedObject var model: SidebarModel
    /// The window controller this sidebar lives in (its document is the
    /// "current" one, and its project scopes the tree).
    let currentDocumentID: ObjectIdentifier
    @ObservedObject var context: WindowSidebarContext
    /// Shared across windows: expansion follows between tabs.
    @ObservedObject var treeState: FileTreeState
    let onSelectDocument: (ObjectIdentifier) -> Void
    /// Opens File Properties for a document: the badge claims to say
    /// what a file is, so it is where you go to correct it.
    let onShowProperties: (ObjectIdentifier) -> Void
    let onOpenFile: (String) -> Void
    /// Moves a project group's windows out into their own window…
    var onSplitGroup: (SidebarProjectGroup) -> Void = { _ in }
    /// …or gathers them into the chosen target window as tabs.
    var onMergeGroup: (SidebarProjectGroup, ObjectIdentifier) -> Void = { _, _ in }
    /// The destinations offered by the Gather Into submenu, computed
    /// when the menu opens.
    var windowTargets: () -> [WindowTarget] = { [] }
    /// The effective hidden-name globs for a project root.
    var hiddenGlobs: (String) -> [String] = { _ in [".*"] }
    /// Expands the tree to a path and highlights it.
    var onRevealInTree: (String) -> Void = { _ in }

    private var projectRoot: String? { context.projectRoot }

    /// Dirty-by-path over every open document, standardized the way
    /// the tree's own keys are.
    private var openFiles: [String: Bool] {
        var open: [String: Bool] = [:]
        for document in model.groups.flatMap(\.documents) {
            guard let path = document.path else { continue }
            open[(path as NSString).standardizingPath] = document.isDirty
        }
        return open
    }

    private var currentPath: String? { context.focusedPath }

    var body: some View {
        // A hand-rolled splitter rather than VSplitView: the divider
        // fraction lives in the shared FileTreeState, so every tab shows
        // the same split instead of each remembering its own.
        GeometryReader { geometry in
            let height = max(geometry.size.height, 1)
            VStack(spacing: 0) {
                bufferPane
                    .frame(height: max(80, (height - 9) * treeState.splitFraction))
                splitDivider(totalHeight: height)
                treePane
                    .frame(maxHeight: .infinity)
            }
            .coordinateSpace(name: "sidebarSplit")
        }
    }

    private func splitDivider(totalHeight: CGFloat) -> some View {
        ZStack {
            Rectangle()
                .fill(Color(nsColor: .separatorColor))
                .frame(height: 1)
        }
        .frame(height: 9)
        .contentShape(Rectangle())
        .onHover { hovering in
            if hovering {
                NSCursor.resizeUpDown.push()
            } else {
                NSCursor.pop()
            }
        }
        .gesture(
            DragGesture(minimumDistance: 1, coordinateSpace: .named("sidebarSplit"))
                .onChanged { value in
                    treeState.splitFraction = min(
                        0.85, max(0.15, value.location.y / totalHeight))
                }
                .onEnded { _ in
                    treeState.onSplitCommitted?()
                }
        )
    }

    private var bufferPane: some View {
        VStack(spacing: 0) {
            HStack {
                Text(t("Open Files"))
                    .font(.caption)
                    .fontWeight(.semibold)
                    .foregroundStyle(.secondary)
                Spacer()
                Toggle(isOn: $model.showFullPaths) {
                    Image(systemName: "list.bullet.indent")
                }
                .toggleStyle(.button)
                .controlSize(.small)
                .help("Show paths from the project root while enabled")
            }
            .padding(.horizontal, 10)
            .padding(.top, 6)
            // Group headers are plain rows rather than Section headers:
            // the sidebar list style unreliably hides the first pinned
            // section header under the scroll inset.
            List {
                if model.groups.isEmpty, !model.settled {
                    Text(t("Loading…"))
                        .foregroundStyle(.secondary)
                }
                ForEach(model.groups) { group in
                    Text(group.name)
                        .font(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(.secondary)
                        .padding(.top, 6)
                        .contentShape(Rectangle())
                        .contextMenu {
                            Button(t("Split into New Window")) {
                                onSplitGroup(group)
                            }
                            Menu(t("Gather Into")) {
                                ForEach(windowTargets()) { target in
                                    Button(target.title) {
                                        onMergeGroup(group, target.id)
                                    }
                                }
                            }
                        }
                    ForEach(group.documents) { document in
                        HStack(spacing: 4) {
                            if document.isDirty {
                                // The dirty dot outranks the badge: unsaved
                                // is the one thing worth noticing here.
                                Image(systemName: "circle.fill")
                                    .font(.system(size: 7))
                                    .foregroundStyle(.primary)
                                    .frame(width: 17)
                            } else {
                                // Clicking the badge asks what the file
                                // is, which is what the badge claims to
                                // answer.
                                FileTypeIcon(filename: document.title)
                                    .contentShape(Rectangle())
                                    .onTapGesture { onShowProperties(document.id) }
                                    .help("File properties")
                            }
                            Text(document.display)
                                .fontWeight(
                                    document.id
                                        == (context.focusedDocumentID ?? currentDocumentID)
                                        ? .semibold : .regular)
                                .lineLimit(1)
                                .truncationMode(.head)
                            Spacer(minLength: 0)
                        }
                        .contentShape(Rectangle())
                        .onTapGesture { onSelectDocument(document.id) }
                        // Dragging a row carries the file itself, so a
                        // drop elsewhere — Slack, a mail, the desktop —
                        // copies or attaches it.
                        .onDrag {
                            guard let path = document.path else { return NSItemProvider() }
                            return NSItemProvider(
                                contentsOf: URL(fileURLWithPath: path)) ?? NSItemProvider()
                        }
                        .contextMenu {
                            Button(t("File Properties…")) {
                                onShowProperties(document.id)
                            }
                            Divider()
                            if let path = document.path {
                                PathCopyMenu(
                                    path: path, projectRoot: group.root,
                                    isDirectory: false,
                                    onReveal: { path in
                                        onRevealInTree(path)
                                    })
                            }
                        }
                    }
                }
            }
            .listStyle(.sidebar)
        }
    }

    private var treePane: some View {
        Group {
            if let projectRoot {
                let rootURL = treeKey(projectRoot, isDirectory: true)
                ScrollViewReader { proxy in
                    List {
                        Section((projectRoot as NSString).lastPathComponent) {
                            if !treeState.hasListing(of: rootURL) {
                                Text(t("Loading…"))
                                    .foregroundStyle(.secondary)
                            }
                            ForEach(
                                treeState.visibleRows(
                                    root: rootURL, globs: hiddenGlobs(projectRoot))
                            ) { row in
                                FileTreeRow(
                                    node: row.node, depth: row.depth,
                                    isExpanded: treeState.expanded.contains(row.node.url),
                                    projectRoot: projectRoot,
                                    openFiles: openFiles, currentPath: currentPath,
                                    highlighted: treeState.highlighted,
                                    onToggle: { url in
                                        if treeState.expanded.contains(url) {
                                            treeState.expanded.remove(url)
                                        } else {
                                            treeState.expanded.insert(url)
                                        }
                                    },
                                    onOpenFile: onOpenFile)
                            }
                        }
                    }
                    .listStyle(.sidebar)
                    .onChange(of: treeState.highlighted) { _, highlighted in
                        if let highlighted {
                            // The ancestors were just expanded; let the
                            // rows exist before scrolling to one.
                            DispatchQueue.main.async {
                                withAnimation { proxy.scrollTo(highlighted) }
                            }
                        }
                    }
                }
            } else {
                Text(t("No project"))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }
}
