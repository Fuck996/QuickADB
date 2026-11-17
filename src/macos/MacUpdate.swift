// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import AppKit
import CryptoKit
import Darwin
import Foundation
import Security

enum MacUpdate {
    private static let repository = "Fuck996/QuickADB"
    private static let releaseURL = URL(string: "https://api.github.com/repos/\(repository)/releases?per_page=50")!
    private static let maximumSize = 128 * 1024 * 1024
    static let restartFlag = "--quickadb-finish-update"
    static let revisionKey = "QuickADBMacBetaRevision"

    struct Version: Comparable, Sendable {
        let major: UInt32
        let minor: UInt32
        let patch: UInt32
        let beta: UInt32

        init?(base: String, beta: UInt32) {
            let components = base.split(separator: ".", omittingEmptySubsequences: false)
            let numbers = components.compactMap { part -> UInt32? in
                guard part.utf8.allSatisfy({ (48...57).contains($0) }),
                      let number = UInt32(part), String(number) == part else { return nil }
                return number
            }
            guard components.count == 3, numbers.count == 3 else { return nil }
            (major, minor, patch) = (numbers[0], numbers[1], numbers[2])
            self.beta = beta
        }

        init?(tag: String) {
            let parts = tag.components(separatedBy: "-mac-beta.")
            guard parts.count == 2, parts[0].first == "v",
                  let beta = UInt32(parts[1]), beta > 0, String(beta) == parts[1] else { return nil }
            self.init(base: String(parts[0].dropFirst()), beta: beta)
        }

        var base: String { "\(major).\(minor).\(patch)" }
        var tag: String { "v\(base)-mac-beta.\(beta)" }

        static func < (lhs: Self, rhs: Self) -> Bool {
            (lhs.major, lhs.minor, lhs.patch, lhs.beta) < (rhs.major, rhs.minor, rhs.patch, rhs.beta)
        }
    }

    struct Release: Sendable {
        let version: Version
        let page: URL
        let download: URL
        let size: Int
        let sha256: String
    }

    struct Staged: Sendable {
        let installed: URL
        let id: UUID
        private var folder: URL { MacUpdate.stageURL(for: installed, id: id) }
        var candidate: URL { folder.appending(path: "QuickADB.app") }

        func swap() throws {
            let result = installed.withUnsafeFileSystemRepresentation { old in
                candidate.withUnsafeFileSystemRepresentation { new in
                    renamex_np(old, new, UInt32(RENAME_SWAP))
                }
            }
            guard result == 0 else {
                throw Failure("无法替换 QuickADB：\(String(cString: strerror(errno)))")
            }
        }

        func discard() {
            try? FileManager.default.removeItem(at: folder)
        }
    }

    private struct Metadata: Decodable {
        let tag_name: String
        let draft: Bool
        let prerelease: Bool
        let assets: [Asset]
    }

    private struct Asset: Decodable {
        let name: String
        let state: String
        let size: Int
        let digest: String?
        let browser_download_url: URL
    }

    struct Failure: LocalizedError {
        let description: String
        init(_ description: String) { self.description = description }
        var errorDescription: String? { description }
    }

    private final class GitHubRedirects: NSObject, URLSessionTaskDelegate {
        func urlSession(_ session: URLSession, task: URLSessionTask,
                        willPerformHTTPRedirection response: HTTPURLResponse,
                        newRequest request: URLRequest,
                        completionHandler: @escaping (URLRequest?) -> Void) {
            let hosts = ["github.com", "api.github.com", "release-assets.githubusercontent.com",
                         "objects.githubusercontent.com"]
            let url = request.url
            completionHandler(url?.scheme == "https" && hosts.contains(url?.host ?? "") ? request : nil)
        }
    }

    private static func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 20
        configuration.timeoutIntervalForResource = 180
        configuration.httpAdditionalHeaders = ["User-Agent": "QuickADB macOS"]
        return URLSession(configuration: configuration, delegate: GitHubRedirects(), delegateQueue: nil)
    }

    static func currentVersion(_ bundle: Bundle) throws -> Version {
        let beta = (bundle.object(forInfoDictionaryKey: revisionKey) as? NSNumber)?.uint32Value ?? 0
        guard let base = bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
              let version = Version(base: base, beta: beta) else {
            throw Failure("无法读取当前 QuickADB 版本。")
        }
        return version
    }

    static func check(current: Version) async throws -> Release? {
        let connection = session()
        defer { connection.finishTasksAndInvalidate() }
        var request = URLRequest(url: releaseURL)
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        let (body, response) = try await connection.data(for: request)
        try checkResponse(response)
        return try selectRelease(from: body, newerThan: current)
    }

    static func selectRelease(from body: Data, newerThan current: Version) throws -> Release? {
        let releases = try JSONDecoder().decode([Metadata].self, from: body)
        let newer = releases.compactMap { release -> (Version, [Asset])? in
            guard release.prerelease, !release.draft,
                  let version = Version(tag: release.tag_name), version > current else { return nil }
            return (version, release.assets)
        }
        guard let (version, assets) = newer.max(by: { $0.0 < $1.0 }) else { return nil }
        let name = "QuickADB-\(version.tag.dropFirst())-arm64.dmg"
        let page = URL(string: "https://github.com/\(repository)/releases/tag/\(version.tag)")!
        let download = URL(string: "https://github.com/\(repository)/releases/download/\(version.tag)/\(name)")!
        guard let asset = assets.first(where: { $0.name == name && $0.state == "uploaded" }),
              asset.browser_download_url == download, (1...maximumSize).contains(asset.size),
              let digest = asset.digest, digest.hasPrefix("sha256:"),
              digest.utf8.count == 71,
              digest.dropFirst(7).utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
            throw Failure("GitHub 发布缺少可验证的 Mac 安装镜像。")
        }
        return Release(version: version, page: page, download: download,
                       size: asset.size, sha256: String(digest.dropFirst(7)))
    }

    static func prepare(_ release: Release, installed: URL) async throws -> Staged {
        let files = FileManager.default
        guard files.isWritableFile(atPath: installed.deletingLastPathComponent().path) else {
            throw Failure("无法写入 QuickADB 所在目录。请将应用拷贝到可写的应用程序目录后重试。")
        }
        let connection = session()
        defer { connection.finishTasksAndInvalidate() }
        let (image, response) = try await connection.download(from: release.download)
        defer { try? files.removeItem(at: image) }
        try checkResponse(response)
        try verify(image: image, release: release)
        return try stage(image: image, release: release, installed: installed)
    }

    private static func checkResponse(_ response: URLResponse) throws {
        guard let response = response as? HTTPURLResponse, response.statusCode == 200 else {
            throw Failure("GitHub 请求失败（HTTP \((response as? HTTPURLResponse)?.statusCode ?? 0)）。")
        }
    }

    static func verify(image: URL, release: Release) throws {
        let file = try FileHandle(forReadingFrom: image)
        defer { try? file.close() }
        var hasher = SHA256()
        var received = 0
        while let bytes = try file.read(upToCount: 256 * 1024), !bytes.isEmpty {
            received += bytes.count
            guard received <= release.size else { throw Failure("安装镜像大小校验失败。") }
            hasher.update(data: bytes)
        }
        guard received == release.size else { throw Failure("安装镜像大小校验失败。") }
        let sha = hasher.finalize().map { String(format: "%02x", $0) }.joined()
        guard sha == release.sha256 else { throw Failure("安装镜像 SHA-256 校验失败。") }
    }

    private static func stage(image: URL, release: Release, installed: URL) throws -> Staged {
        let files = FileManager.default
        let mount = files.temporaryDirectory.appending(path: "QuickADB-image-\(UUID().uuidString)")
        try files.createDirectory(at: mount, withIntermediateDirectories: false)
        var mounted = false
        defer {
            if !mounted || (try? diskImage(["detach", mount.path])) != nil {
                try? files.removeItem(at: mount)
            }
        }
        try diskImage(["attach", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", mount.path, image.path])
        mounted = true
        let app = mount.appending(path: "QuickADB.app")
        let resource = try app.resourceValues(forKeys: [.isDirectoryKey, .isSymbolicLinkKey])
        guard resource.isDirectory == true, resource.isSymbolicLink != true else {
            throw Failure("镜像中的 QuickADB 应用无效。")
        }
        let staged = Staged(installed: installed, id: UUID())
        let folder = stageURL(for: installed, id: staged.id)
        try files.createDirectory(at: folder, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        do {
            try files.copyItem(at: app, to: staged.candidate)
            try validateBundle(staged.candidate, version: release.version, installed: installed)
        } catch {
            staged.discard()
            throw error
        }
        return staged
    }

    private static func diskImage(_ arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/hdiutil")
        process.arguments = arguments
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw Failure("无法\(arguments[0] == "attach" ? "挂载" : "卸载")更新镜像。")
        }
    }

    private static func validateBundle(_ candidate: URL, version: Version, installed: URL) throws {
        guard let bundle = Bundle(url: candidate), bundle.bundleIdentifier == "io.github.fuck996.QuickADB",
              let executable = bundle.executableURL,
              bundle.executableArchitectures?.contains(NSNumber(value: NSBundleExecutableArchitectureARM64)) == true else {
            throw Failure("镜像中的 QuickADB 与此 Mac 不兼容。")
        }
        let bundledVersion = try currentVersion(bundle)
        // The first published beta predates the revision key in Info.plist.
        let revisionMatches = bundledVersion.beta == version.beta || (bundledVersion.beta == 0 && version.beta == 1)
        guard bundledVersion.base == version.base, revisionMatches,
              executable.path == candidate.appending(path: "Contents/MacOS/QuickADB").path else {
            throw Failure("镜像中的应用版本与发布标签不一致。")
        }
        let code = try staticCode(candidate)
        guard SecStaticCodeCheckValidity(code, SecCSFlags(rawValue: UInt32(kSecCSStrictValidate)), nil) == errSecSuccess else {
            throw Failure("新版 QuickADB 的代码签名校验失败。")
        }
        if let team = try signingTeam(staticCode(installed)), try signingTeam(code) != team {
            throw Failure("新版 QuickADB 的签名团队与当前应用不一致。")
        }
    }

    private static func staticCode(_ url: URL) throws -> SecStaticCode {
        var code: SecStaticCode?
        guard SecStaticCodeCreateWithPath(url as CFURL, SecCSFlags(), &code) == errSecSuccess,
              let code else { throw Failure("无法读取 QuickADB 的代码签名。") }
        return code
    }

    private static func signingTeam(_ code: SecStaticCode) throws -> String? {
        var info: CFDictionary?
        guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: UInt32(kSecCSSigningInformation)), &info) == errSecSuccess,
              let info = info as? [String: Any] else { throw Failure("无法确认 QuickADB 的签名团队。") }
        return info[kSecCodeInfoTeamIdentifier as String] as? String
    }

    private static func stageURL(for installed: URL, id: UUID) -> URL {
        installed.deletingLastPathComponent().appending(path: ".QuickADB-update-\(id.uuidString)")
    }

    @MainActor
    static func relaunch(_ staged: Staged) async throws -> NSRunningApplication {
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.createsNewApplicationInstance = true
        configuration.allowsRunningApplicationSubstitution = false
        configuration.arguments = [restartFlag, staged.id.uuidString]
        return try await NSWorkspace.shared.openApplication(at: staged.installed, configuration: configuration)
    }

    static func finishPreviousUpdate(installed: URL, arguments: [String]) {
        guard let flag = arguments.dropFirst().firstIndex(of: restartFlag),
              arguments.indices.contains(flag + 1), let id = UUID(uuidString: arguments[flag + 1]) else { return }
        Staged(installed: installed, id: id).discard()
    }
}
