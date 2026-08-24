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

    /// Rebuilds the grouped buffer list. `entries` pairs each document
    /// with its (cached) project root.
    func rebuild(entries: [(document: SidebarDocument, projectRoot: String?)]) {
        var byRoot: [String?: [SidebarDocument]] = [:]
        for entry in entries {
            byRoot[entry.projectRoot, default: []].append(entry.document)
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
    let onSelectDocument: (ObjectIdentifier) -> Void
    let onOpenFile: (String) -> Void

    private var projectRoot: String? { context.projectRoot }

    var body: some View {
        VSplitView {
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
                    ForEach(group.documents) { document in
                        HStack(spacing: 4) {
                            Image(
                                systemName: document.isDirty
                                    ? "circle.fill" : "doc.text"
                            )
                            .font(.system(size: document.isDirty ? 7 : 12))
                            .foregroundStyle(
                                document.isDirty ? .primary : .secondary)
                            Text(document.title)
                                .fontWeight(
                                    document.id == currentDocumentID ? .semibold : .regular)
                            Spacer(minLength: 0)
                        }
                        .contentShape(Rectangle())
                        .onTapGesture { onSelectDocument(document.id) }
                    }
                }
            }
            .listStyle(.sidebar)
            .frame(minHeight: 120)

            Group {
                if let projectRoot {
                    let rootNode = FileNode(
                        url: URL(fileURLWithPath: projectRoot), isDirectory: true)
                    List {
                        Section((projectRoot as NSString).lastPathComponent) {
                            OutlineGroup(rootNode.children ?? [], children: \.children) { node in
                                HStack(spacing: 4) {
                                    Image(systemName: node.isDirectory ? "folder" : "doc.text")
                                        .foregroundStyle(.secondary)
                                    Text(node.name)
                                    Spacer(minLength: 0)
                                }
                                .contentShape(Rectangle())
                                .onTapGesture {
                                    if !node.isDirectory { onOpenFile(node.url.path) }
                                }
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
            .frame(minHeight: 120)
        }
    }
}
