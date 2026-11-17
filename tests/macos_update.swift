// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import CryptoKit
import Foundation

@main
struct MacUpdateTests {
    static func main() throws {
        let repository = "https://github.com/Fuck996/QuickADB"
        let current = MacUpdate.Version(tag: "v0.9.10-mac-beta.2")!
        let content = Data("QuickADB update fixture".utf8)
        let digest = SHA256.hash(data: content).map { String(format: "%02x", $0) }.joined()
        func metadata(_ tag: String, prerelease: Bool, digest: String = digest) -> [String: Any] {
            let name = "QuickADB-\(tag.dropFirst())-arm64.dmg"
            return [
                "tag_name": tag,
                "draft": false,
                "prerelease": prerelease,
                "assets": [[
                    "name": name, "size": content.count, "state": "uploaded",
                    "digest": "sha256:\(digest)",
                    "browser_download_url": "\(repository)/releases/download/\(tag)/\(name)",
                ]],
            ]
        }
        let newer = metadata("v0.9.10-mac-beta.11", prerelease: true)
        let other = [
            metadata("v0.9.11", prerelease: false),
            metadata("v0.9.10-mac-beta.1", prerelease: true),
            metadata("v0.9.10-mac-beta.2", prerelease: true),
        ]
        let listing = try JSONSerialization.data(withJSONObject: [newer] + other)
        guard let release = try MacUpdate.selectRelease(from: listing, newerThan: current),
              release.version.tag == "v0.9.10-mac-beta.11" else { fatalError("wrong update chosen") }
        let noUpdate = try MacUpdate.selectRelease(
            from: JSONSerialization.data(withJSONObject: other), newerThan: current
        )
        precondition(noUpdate == nil)
        let malformed = try JSONSerialization.data(withJSONObject: [
            metadata("v0.9.11-mac-beta.1", prerelease: true, digest: "invalid"),
        ])
        do {
            _ = try MacUpdate.selectRelease(from: malformed, newerThan: current)
            fatalError("accepted an invalid digest")
        } catch is MacUpdate.Failure {}

        let files = FileManager.default
        let root = files.temporaryDirectory.appending(path: "quickadb-update-tests-\(UUID())")
        try files.createDirectory(at: root, withIntermediateDirectories: false)
        defer { try? files.removeItem(at: root) }
        let image = root.appending(path: "fixture.dmg")
        try content.write(to: image)
        try MacUpdate.verify(image: image, release: release)
        try Data(repeating: 0, count: content.count).write(to: image)
        do {
            try MacUpdate.verify(image: image, release: release)
            fatalError("accepted an altered payload")
        } catch is MacUpdate.Failure {}

        let installed = root.appending(path: "QuickADB.app")
        let staged = MacUpdate.Staged(installed: installed, id: UUID())
        try files.createDirectory(at: installed, withIntermediateDirectories: false)
        try files.createDirectory(at: staged.candidate, withIntermediateDirectories: true)
        let marker = "old-version"
        try content.write(to: installed.appending(path: marker))
        try staged.swap()
        precondition(files.fileExists(atPath: staged.candidate.appending(path: marker).path))
        try staged.swap()
        precondition(files.fileExists(atPath: installed.appending(path: marker).path))
        MacUpdate.finishPreviousUpdate(installed: installed, arguments: [
            "QuickADB", MacUpdate.restartFlag, staged.id.uuidString,
        ])
        precondition(!files.fileExists(atPath: staged.candidate.path))
        print("Mac update selection, digest rejection, swap, rollback and cleanup passed")
    }
}
