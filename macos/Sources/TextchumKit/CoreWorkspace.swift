import CTextchum
import Foundation

/// The core's workspace model: which project a file belongs to.
public enum CoreWorkspace {
    /// The project root for a file or directory path — the nearest
    /// ancestor with a root marker (VCS directory, build/manifest file) —
    /// or nil for loose files outside any project.
    public static func projectRoot(forPath path: String) -> String? {
        var path = path
        let cString: UnsafeMutablePointer<CChar>? = path.withUTF8 { bytes in
            let pointer = bytes.baseAddress.map {
                UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
            }
            return tc_project_root_for_path(pointer, UInt(bytes.count))
        }
        guard let cString else { return nil }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }
}
