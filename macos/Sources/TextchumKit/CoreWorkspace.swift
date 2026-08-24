import CTextchum
import Foundation

/// The core's workspace model: which project a file belongs to.
public enum CoreWorkspace {
    /// The project root for a file or directory path, resolved under the
    /// given workspace settings JSON (the configuration's `workspace`
    /// section; "{}" for defaults) — or nil for loose files outside any
    /// project.
    public static func projectRoot(forPath path: String, settingsJSON: String = "{}") -> String?
    {
        var path = path
        var settings = settingsJSON
        let cString: UnsafeMutablePointer<CChar>? = path.withUTF8 { pathBytes in
            settings.withUTF8 { settingsBytes in
                tc_project_root_for_path(
                    pathBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(pathBytes.count),
                    settingsBytes.baseAddress.map {
                        UnsafeRawPointer($0).assumingMemoryBound(to: CChar.self)
                    },
                    UInt(settingsBytes.count)
                )
            }
        }
        guard let cString else { return nil }
        defer { tc_string_free(cString) }
        return String(cString: cString)
    }
}
