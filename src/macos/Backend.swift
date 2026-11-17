// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import Foundation
import Observation
import QuickADBBridge

struct Endpoint: Decodable, Equatable {
    let host: String
    let port: UInt16

    var address: String { host.contains(":") ? "[\(host)]:\(port)" : "\(host):\(port)" }
}

struct Device: Decodable, Equatable, Identifiable {
    let id: String
    let name: String
    let model: String
    let serial: String
    let android: String
    let transport: String
    let status: String
    let online: Bool
    let selected: Bool
    let selectable: Bool
    let managing: Bool
    let detail: String
    let endpoint: Endpoint?
    let pairedId: String?
    let alias: String?

    var wireless: Bool { endpoint != nil || pairedId != nil }
}

struct Job: Decodable, Equatable, Identifiable {
    enum Stage: String, Decodable {
        case transferring, installing, succeeded, canceled, failed, waiting
    }

    let id: UInt64
    let title: String
    let device: String
    let status: String
    let stage: Stage
    let active: Bool
    let transferred: UInt64
    let total: UInt64
    let speed: Double
    let detail: String
}

struct Preparation: Decodable, Equatable {
    enum Status: String, Decodable { case checking, ready, failed }

    struct File: Decodable, Equatable, Identifiable {
        let path: String
        let size: UInt64?
        let package: String?
        let split: String?
        var id: String { path }
        var name: String { URL(fileURLWithPath: path).lastPathComponent }
    }

    let revision: UInt64
    let split: Bool
    let status: Status
    let files: [File]
    let error: String?
}

struct Service: Decodable, Equatable, Identifiable {
    enum Kind: String, Decodable {
        case pairing, tls, tcp

        var title: String {
            switch self {
            case .pairing: "配对服务"
            case .tls: "无线调试"
            case .tcp: "传统 TCP"
            }
        }
    }

    let id: String
    let name: String
    let kind: Kind
    let endpoint: Endpoint
    let pairedId: String?
}

struct Snapshot: Decodable {
    var devices: [Device] = []
    var jobs: [Job] = []
    var preparation: Preparation?
    var discovered: [Service] = []
    var notice = ""
    var connecting = false
    var pairing = false
    var pinned = false
    var testPackages = false
}

struct Command: Encodable {
    enum Action: String, Encodable {
        case toggle, prepare, install, cancel, pair, connect, disconnect, forget, alias, preferences
        case selectAll = "select_all"
        case clearPreparation = "clear_preparation"
        case retryJob = "retry_job"
        case clearFinished = "clear_finished"
        case clearNotice = "clear_notice"
        case connectPaired = "connect_paired"
        case retryConnection = "retry_connection"
        case editConnection = "edit_connection"
        case enableTcp = "enable_tcp"
    }

    let action: Action
    var id: String? = nil
    var jobId: UInt64? = nil
    var revision: UInt64? = nil
    var paths: [String]? = nil
    var split: Bool? = nil
    var selected: Bool? = nil
    var host: String? = nil
    var port: UInt16? = nil
    var code: String? = nil
    var pairedId: String? = nil
    var name: String? = nil
    var pinned: Bool? = nil
    var testPackages: Bool? = nil

    init(
        _ action: Action, id: String? = nil, jobId: UInt64? = nil, revision: UInt64? = nil,
        paths: [String]? = nil, split: Bool? = nil, selected: Bool? = nil, host: String? = nil,
        port: UInt16? = nil, code: String? = nil, pairedId: String? = nil, name: String? = nil,
        pinned: Bool? = nil, testPackages: Bool? = nil
    ) {
        self.action = action
        self.id = id
        self.jobId = jobId
        self.revision = revision
        self.paths = paths
        self.split = split
        self.selected = selected
        self.host = host
        self.port = port
        self.code = code
        self.pairedId = pairedId
        self.name = name
        self.pinned = pinned
        self.testPackages = testPackages
    }

}

private struct BackendFailure: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

private final class RustBackend {
    let handle: OpaquePointer

    init(directory: URL) throws {
        var error: UnsafeMutablePointer<CChar>?
        guard let handle = directory.path.withCString({ quickadb_open($0, &error) }) else {
            throw BackendFailure(message: error.map(Self.takeString) ?? "无法启动 ADB 服务")
        }
        self.handle = handle
    }

    static func takeString(_ pointer: UnsafeMutablePointer<CChar>) -> String {
        defer { quickadb_string_free(pointer) }
        return String(cString: pointer)
    }

    deinit { quickadb_close(handle) }
}

@MainActor @Observable
final class BackendModel {
    let directory: URL
    private let backend: RustBackend
    private let decoder = JSONDecoder()
    private let encoder = JSONEncoder()
    private var lastPayload = Data()
    private(set) var snapshot = Snapshot()
    var actionError: String?
    var restartingForUpdate = false

    init() throws {
        directory = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        ).appending(path: "QuickADB", directoryHint: .isDirectory)
        backend = try RustBackend(directory: directory)
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        encoder.keyEncodingStrategy = .convertToSnakeCase
        refresh()
    }

    func refresh() {
        let pointer = quickadb_snapshot(backend.handle)
        defer { quickadb_string_free(pointer) }
        let payload = Data(bytes: pointer, count: strlen(pointer))
        guard payload != lastPayload else { return }
        lastPayload = payload
        do {
            snapshot = try decoder.decode(Snapshot.self, from: payload)
        } catch {
            actionError = "无法读取任务与设备状态：\(error.localizedDescription)"
        }
    }

    @discardableResult
    func send(_ command: Command) -> Bool {
        guard !restartingForUpdate else {
            actionError = "QuickADB 正在安装更新并重新打开。"
            return false
        }
        do {
            let data = try encoder.encode(command)
            let payload = String(decoding: data, as: UTF8.self)
            if let error = payload.withCString({ quickadb_command(backend.handle, $0) }) {
                throw BackendFailure(message: RustBackend.takeString(error))
            }
            actionError = nil
            refresh()
            return true
        } catch {
            actionError = error.localizedDescription
            return false
        }
    }

    func prepare(_ urls: [URL], split: Bool) {
        let paths = urls.filter(\.isFileURL).map(\.path)
        guard !paths.isEmpty else { return }
        send(Command(.prepare, paths: paths, split: split))
    }
}
