import AppKit
import SwiftUI
import TextchumKit

extension Notification.Name {
    /// Posted whenever the set of open documents, or any document's path,
    /// dirty state, or title changes — the sidebar rebuilds from it.
    static let textchumDocumentsChanged = Notification.Name("textchumDocumentsChanged")
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

    /// Fraction of the sidebar's height given to the buffer list.
    /// Shared like the expansion set, so every tab shows the same split
    /// instead of each remembering its own; persisted with the session.
    @Published var splitFraction: Double = 0.45
    /// Called when a divider drag ends, so the session can be saved
    /// once, not per drag tick.
    var onSplitCommitted: (() -> Void)?

    func binding(for url: URL) -> Binding<Bool> {
        Binding(
            get: { self.expanded.contains(url) },
            set: { open in
                if open {
                    self.expanded.insert(url)
                } else {
                    self.expanded.remove(url)
                }
            }
        )
    }
}

/// One row of the project tree; directories recurse via disclosure
/// groups whose expansion lives in the shared ``FileTreeState``.
struct FileTreeRow: View {
    let node: FileNode
    @ObservedObject var state: FileTreeState
    let projectRoot: String?
    let onOpenFile: (String) -> Void

    var body: some View {
        if node.isDirectory {
            DisclosureGroup(isExpanded: state.binding(for: node.url)) {
                ForEach(node.children ?? []) { child in
                    FileTreeRow(
                        node: child, state: state, projectRoot: projectRoot,
                        onOpenFile: onOpenFile)
                }
            } label: {
                label
            }
        } else {
            label
                .contentShape(Rectangle())
                .onTapGesture { onOpenFile(node.url.path) }
        }
    }

    private var label: some View {
        HStack(spacing: 4) {
            Image(systemName: node.isDirectory ? "folder" : "doc.text")
                .foregroundStyle(.secondary)
            Text(node.name)
            Spacer(minLength: 0)
        }
        .contextMenu {
            PathCopyMenu(
                path: node.url.path, projectRoot: projectRoot,
                isDirectory: node.isDirectory)
        }
    }
}

/// A file-system node of the project tree. Children are read lazily from
/// disk on expansion; hidden files are skipped.
struct FileNode: Identifiable, Hashable {
    let url: URL
    let isDirectory: Bool

    var id: URL { url }
    var name: String { url.lastPathComponent }

    /// nil for files (so OutlineGroup shows no disclosure), the sorted
    /// visible entries for directories.
    var children: [FileNode]? {
        guard isDirectory else { return nil }
        let entries = (try? FileManager.default.contentsOfDirectory(
            at: url,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        )) ?? []
        return entries
            .map { url in
                FileNode(
                    url: url,
                    isDirectory: (try? url.resourceValues(forKeys: [.isDirectoryKey]))?
                        .isDirectory ?? false
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
    let onOpenFile: (String) -> Void
    /// Moves a project group's windows out into their own window…
    var onSplitGroup: (SidebarProjectGroup) -> Void = { _ in }
    /// …or gathers them into the chosen target window as tabs.
    var onMergeGroup: (SidebarProjectGroup, ObjectIdentifier) -> Void = { _, _ in }
    /// The destinations offered by the Gather Into submenu, computed
    /// when the menu opens.
    var windowTargets: () -> [WindowTarget] = { [] }

    private var projectRoot: String? { context.projectRoot }

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
                Text("Open Files")
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
                ForEach(model.groups) { group in
                    Text(group.name)
                        .font(.caption)
                        .fontWeight(.semibold)
                        .foregroundStyle(.secondary)
                        .padding(.top, 6)
                        .contentShape(Rectangle())
                        .contextMenu {
                            Button("Split into New Window") {
                                onSplitGroup(group)
                            }
                            Menu("Gather Into") {
                                ForEach(windowTargets()) { target in
                                    Button(target.title) {
                                        onMergeGroup(group, target.id)
                                    }
                                }
                            }
                        }
                    ForEach(group.documents) { document in
                        HStack(spacing: 4) {
                            Image(
                                systemName: document.isDirty
                                    ? "circle.fill" : "doc.text"
                            )
                            .font(.system(size: document.isDirty ? 7 : 12))
                            .foregroundStyle(
                                document.isDirty ? .primary : .secondary)
                            Text(document.display)
                                .fontWeight(
                                    document.id == currentDocumentID
                                        ? .semibold : .regular)
                                .lineLimit(1)
                                .truncationMode(.head)
                            Spacer(minLength: 0)
                        }
                        .contentShape(Rectangle())
                        .onTapGesture { onSelectDocument(document.id) }
                        .contextMenu {
                            if let path = document.path {
                                PathCopyMenu(
                                    path: path, projectRoot: group.root,
                                    isDirectory: false)
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
                let rootNode = FileNode(
                    url: URL(fileURLWithPath: projectRoot), isDirectory: true)
                List {
                    Section((projectRoot as NSString).lastPathComponent) {
                        ForEach(rootNode.children ?? []) { node in
                            FileTreeRow(
                                node: node, state: treeState,
                                projectRoot: projectRoot, onOpenFile: onOpenFile)
                        }
                    }
                }
                .listStyle(.sidebar)
            } else {
                Text("No project")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }
}
