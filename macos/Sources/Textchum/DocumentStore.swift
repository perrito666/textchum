import Foundation
import TextchumKit

/// One open document — what the file is, not where it is shown.
///
/// Views come and go; this is what they are views of. Everything here
/// belongs to the document, so two views of one file cannot disagree
/// about it.
final class OpenDocument {
    /// Stable, and independent of where the document is shown or
    /// whether it has a path yet.
    let id: Int
    /// The text, history and syntax, owned by the core.
    let core: CoreDocument
    /// The path the store knows this document by, which is what the
    /// index and the recently closed cache go on. A document saved
    /// under a new name keeps its identity and changes this.
    var path: String?
    /// How this file is shown: how many views a column stacks of it,
    /// where the dividers between them sit, and where each view was
    /// looking. It belongs to the file, so a column switched to
    /// another tab and back finds it as it was.
    var layout = DocumentLayout()
    /// The stretches folded shut, as first and last line, both
    /// zero-based. They belong to the document: folding a function in
    /// one view folds it in every view of the file.
    var folds: [(start: Int, end: Int)] = []
    /// What the server last said about it. A finding is about the
    /// file, not about the window it happens to be visible in.
    var diagnostics: [CoreDiagnostic] = []

    init(id: Int, core: CoreDocument) {
        self.id = id
        self.core = core
    }
}

/// How a file is shown, wherever it is shown. Values only, so a
/// document can hold one without being tied to the main thread.
struct DocumentLayout {
    /// Views stacked in the column showing it, at least one.
    var views = 1
    /// Where the dividers between them sit, as fractions of the
    /// column's height.
    var dividers: [Double] = []
    /// Where each view was looking: the caret in UTF-16 units, and the
    /// scroll in points.
    var places: [Place] = []

    struct Place {
        var caret = 0
        var scroll = 0.0
        /// The first character shown: the line, which a reflow keeps
        /// where the pixel offset would not.
        var top = 0
    }
}

/// Every open document, and the paths that name them.
///
/// A document is here once however many views show it, which is what
/// lets a second view be an ordinary thing rather than a special case.
@MainActor
final class DocumentStore {
    static let shared = DocumentStore()

    private var documents: [Int: OpenDocument] = [:]
    private var byPath: [String: Int] = [:]
    private var nextID = 1

    /// Registers a document and hands back its entry. A path already
    /// open gives back the entry it already has.
    @discardableResult
    func open(_ core: CoreDocument, path: String?) -> OpenDocument {
        if let path, let existing = document(forPath: path) {
            return existing
        }
        let document = OpenDocument(id: nextID, core: core)
        document.path = path
        nextID += 1
        documents[document.id] = document
        if let path {
            byPath[path] = document.id
        }
        return document
    }

    func document(id: Int) -> OpenDocument? {
        documents[id]
    }

    /// The document a path names, while it is open.
    /// Documents whose last view closed, newest last. A closed file
    /// is kept whole for a while so that reopening it is taking the
    /// closing back rather than reading the file again.
    private var closed: [OpenDocument] = []

    /// Deep enough to undo a run of mistaken closes, shallow enough
    /// that the list stays a list of recent mistakes.
    private static let closedMemory = 20

    func document(forPath path: String) -> OpenDocument? {
        byPath[path].flatMap { documents[$0] }
    }

    /// Follows a document that has just been given a path, or a new
    /// one — the index is by path, and the path moved.
    func rename(_ id: Int, from: String?, to: String) {
        if let from { byPath.removeValue(forKey: from) }
        byPath[to] = id
        documents[id]?.path = to
    }

    /// Closes a document — its last view has gone — and keeps it in
    /// the recently closed cache. Reopening the file within the next
    /// twenty closings gets the same document back, with anything
    /// typed since the last save.
    ///
    /// Whether unsaved changes were saved or given up is settled
    /// before this is called.
    func close(_ id: Int) {
        let closing = documents[id]
        documents.removeValue(forKey: id)
        byPath = byPath.filter { $0.value != id }
        guard let closing else { return }
        closed.removeAll { $0.id == id }
        closed.append(closing)
        if closed.count > Self.closedMemory {
            closed.removeFirst(closed.count - Self.closedMemory)
        }
    }

    /// Takes a closed document back out of the cache and opens it
    /// again, or answers nil when the file was not closed recently.
    func reclaim(path: String) -> OpenDocument? {
        guard let at = closed.firstIndex(where: { $0.path == path }) else {
            return nil
        }
        let document = closed.remove(at: at)
        documents[document.id] = document
        byPath[path] = document.id
        return document
    }

    /// Every document with changes that were never saved, open or put
    /// aside — what the editor has to settle before it closes.
    var unsaved: [OpenDocument] {
        (Array(documents.values) + closed).filter { $0.core.isDirty }
    }

    /// How many are in the recently closed cache, for the tests.
    var closedCount: Int { closed.count }

    /// How many are open, for the tests.
    var count: Int { documents.count }
}
