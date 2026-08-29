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
    /// What the server last said about it. A finding is about the
    /// file, not about the window it happens to be visible in.
    var diagnostics: [CoreDiagnostic] = []

    init(id: Int, core: CoreDocument) {
        self.id = id
        self.core = core
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
    func document(forPath path: String) -> OpenDocument? {
        byPath[path].flatMap { documents[$0] }
    }

    /// Follows a document that has just been given a path, or a new
    /// one — the index is by path, and the path moved.
    func rename(_ id: Int, from: String?, to: String) {
        if let from { byPath.removeValue(forKey: from) }
        byPath[to] = id
    }

    /// Forgets a document. Its views are gone by the time this is
    /// called; what happens to one with unsaved changes is the
    /// caller's business.
    func close(_ id: Int) {
        documents.removeValue(forKey: id)
        byPath = byPath.filter { $0.value != id }
    }

    /// How many are open, for the tests.
    var count: Int { documents.count }
}
