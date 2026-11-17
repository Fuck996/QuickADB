// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import AppKit
import Observation
import SwiftUI
import UniformTypeIdentifiers

enum WorkspaceSheet: Identifiable {
    case connection
    case device(String)
    case job(UInt64)

    var id: String {
        switch self {
        case .connection: "connection"
        case .device(let id): "device:\(id)"
        case .job(let id): "job:\(id)"
        }
    }
}

@MainActor @Observable
final class WorkspaceRoutes {
    var sheet: WorkspaceSheet?
}

struct Workspace: View {
    let model: BackendModel
    @Bindable var routes: WorkspaceRoutes
    var drawer = false
    var openWindow: () -> Void
    var togglePin: () -> Void
    var openSettings: () -> Void
    var drawerSheet: ((WorkspaceSheet) -> Void)? = nil

    var body: some View {
        Group {
            if drawer {
                drawerContent
            } else {
                NavigationSplitView {
                    List {
                        Section("设备") { deviceRows }
                    }
                    .listStyle(.sidebar)
                    .navigationSplitViewColumnWidth(min: 210, ideal: 244, max: 320)
                    .safeAreaInset(edge: .bottom, spacing: 0) {
                        deviceControls.padding(12)
                    }
                } detail: {
                    VStack(spacing: 0) {
                        ScrollView {
                            preparation.padding(24)
                        }
                        .frame(minHeight: 240, idealHeight: 300, maxHeight: 310)
                        Divider()
                        jobs
                        notice
                    }
                }
            }
        }
        .sheet(item: $routes.sheet) { sheet in
            switch sheet {
            case .connection:
                ConnectionSheet(model: model)
            case .device(let id):
                DeviceSheet(model: model, id: id)
            case .job(let id):
                JobSheet(model: model, id: id)
            }
        }
    }

    private var drawerContent: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Text("QuickADB").font(.headline)
                Spacer()
                Button(action: openWindow) {
                    Label("打开主窗口", systemImage: "macwindow")
                }
                Button(action: togglePin) {
                    Label(model.snapshot.pinned ? "取消固定" : "固定抽屉",
                          systemImage: model.snapshot.pinned ? "pin.fill" : "pin")
                }
                Button(action: openSettings) {
                    Label("设置", systemImage: "gearshape")
                }
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.glass)
            .padding(14)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    VStack(alignment: .leading, spacing: 9) {
                        HStack {
                            Text("设备").font(.headline)
                            Spacer()
                            deviceControls
                        }
                        deviceRows
                    }
                    Divider()
                    preparation
                    Divider()
                    jobHeader
                    if model.snapshot.jobs.isEmpty {
                        Text("暂无安装任务").foregroundStyle(.secondary)
                    } else {
                        ForEach(model.snapshot.jobs) { job in jobRow(job) }
                    }
                }
                .padding(16)
            }
            notice
        }
        .frame(width: 400, height: 640)
    }

    private var deviceControls: some View {
        HStack {
            let selectable = model.snapshot.devices.filter(\.selectable)
            let allSelected = !selectable.isEmpty && selectable.allSatisfy(\.selected)
            if selectable.count > 1 {
                Button(allSelected ? "取消全选" : "全选") {
                    model.send(Command(.selectAll, selected: !allSelected))
                }
                .buttonStyle(.borderless)
            }
            Spacer(minLength: 8)
            Button {
                present(.connection)
            } label: {
                Label("连接设备", systemImage: "plus")
            }
            .help("无线配对或连接")
        }
        .controlSize(.small)
    }

    @ViewBuilder private var deviceRows: some View {
        if model.snapshot.devices.isEmpty {
            VStack(alignment: .leading, spacing: 5) {
                Label("暂无设备", systemImage: "iphone")
                Text("连接 USB 数据线，或添加无线设备。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.vertical, 8)
        } else {
            ForEach(model.snapshot.devices) { device in
                HStack(spacing: 9) {
                    Toggle(isOn: Binding(get: { device.selected }, set: { _ in
                        model.send(Command(.toggle, id: device.id))
                    })) {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(device.name).lineLimit(1)
                            HStack(spacing: 4) {
                                Image(systemName: device.online ? "circle.fill" : "circle")
                                    .font(.system(size: 7))
                                    .foregroundStyle(device.online ? Color.green : .secondary)
                                    .accessibilityHidden(true)
                                Text("\(device.transport) · \(device.status)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                            }
                        }
                    }
                    .toggleStyle(.checkbox)
                    .disabled(!device.selectable || device.managing)
                    Spacer(minLength: 0)
                    Button {
                        present(.device(device.id))
                    } label: {
                        Label("设备详情", systemImage: "info.circle").labelStyle(.iconOnly)
                    }
                    .buttonStyle(.borderless)
                    .help("查看 \(device.name) 的连接详情")
                }
                .padding(.vertical, 4)
            }
        }
    }

    private var preparation: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text("待安装").font(drawer ? .headline : .title2.weight(.semibold))
                Spacer()
                if model.snapshot.preparation != nil {
                    Button("清空") { model.send(Command(.clearPreparation)) }
                        .buttonStyle(.borderless)
                }
            }
            DropZone(model: model, compact: drawer)
            if let prepared = model.snapshot.preparation {
                preparationState(prepared)
                ForEach(prepared.files) { file in
                    HStack(alignment: .center, spacing: 8) {
                        Image(systemName: "shippingbox").foregroundStyle(.secondary)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(file.name).lineLimit(1).truncationMode(.middle).help(file.path)
                            if let package = file.package {
                                Text(file.split.map { $0.isEmpty ? package : "\(package) · \($0)" } ?? package)
                                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                            }
                        }
                        Spacer(minLength: 4)
                        if let size = file.size {
                            Text(byteCount(size)).font(.caption).foregroundStyle(.secondary)
                        }
                        Button {
                            let paths = prepared.files.filter { $0.id != file.id }.map(\.path)
                            model.send(Command(.prepare, paths: paths, split: prepared.split))
                        } label: {
                            Label("移除 \(file.name)", systemImage: "minus.circle").labelStyle(.iconOnly)
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
            HStack {
                Button("选择 APK…") { pickAPKs(model, split: false) }
                Button("选择拆分 APK…") { pickAPKs(model, split: true) }
                Spacer(minLength: 6)
                let selected = model.snapshot.devices.count { $0.selected && $0.selectable }
                Button(selected > 1 ? "安装到 \(selected) 台" : "安装") {
                    if let item = model.snapshot.preparation {
                        model.send(Command(.install, revision: item.revision))
                    }
                }
                .disabled(selected == 0 || model.snapshot.preparation?.status != .ready)
                .buttonStyle(.borderedProminent)
            }
            if model.snapshot.preparation?.status == .ready,
               !model.snapshot.devices.contains(where: { $0.selected && $0.selectable }) {
                Text("请先选择安装目标。").font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder private func preparationState(_ prepared: Preparation) -> some View {
        HStack {
            Picker("安装方式", selection: Binding(get: { prepared.split }, set: { value in
                model.send(Command(.prepare, paths: prepared.files.map(\.path), split: value))
            })) {
                Text("独立 APK").tag(false)
                Text("拆分 APK").tag(true)
            }
            .labelsHidden()
            .fixedSize()
            Spacer()
            if prepared.status == .checking {
                ProgressView().controlSize(.small)
                Text("检查 APK").foregroundStyle(.secondary)
            } else if prepared.status == .ready {
                Label("检查通过", systemImage: "checkmark.circle").foregroundStyle(.secondary)
            }
        }
        if let error = prepared.error {
            Label(error, systemImage: "exclamationmark.triangle")
                .font(.callout)
                .foregroundStyle(.red)
                .textSelection(.enabled)
        }
    }

    private var jobs: some View {
        VStack(alignment: .leading, spacing: 0) {
            jobHeader.padding(.horizontal, 24).padding(.vertical, 14)
            if model.snapshot.jobs.isEmpty {
                ContentUnavailableView("暂无安装任务", systemImage: "tray",
                                       description: Text("安装提交后，进度和结果会显示在这里。"))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(model.snapshot.jobs) { job in
                    jobRow(job).padding(.vertical, 4)
                }
                .listStyle(.inset)
                .frame(maxHeight: .infinity)
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private var jobHeader: some View {
        HStack {
            Text("安装任务").font(drawer ? .headline : .title3.weight(.semibold))
            Spacer()
            if model.snapshot.jobs.contains(where: { !$0.active }) {
                Button("清理已结束") { model.send(Command(.clearFinished)) }
                    .buttonStyle(.borderless)
            }
        }
    }

    private func jobRow(_ job: Job) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: statusSymbol(job.stage))
                .foregroundStyle(statusColor(job.stage))
                .frame(width: 19)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(job.title).fontWeight(.medium).lineLimit(1).truncationMode(.middle).help(job.title)
                Text("\(job.device) · \(job.status)")
                    .font(.caption).foregroundStyle(.secondary)
                if job.stage == .transferring && job.total > 0 {
                    ProgressView(value: Double(job.transferred), total: Double(job.total))
                        .accessibilityLabel("APK 传输进度")
                    HStack {
                        Text("\(byteCount(job.transferred)) / \(byteCount(job.total))")
                        Spacer()
                        Text("\(Int(Double(job.transferred) * 100 / Double(job.total)))%")
                        Text("\(byteCount(UInt64(max(job.speed, 0))))/秒")
                    }
                    .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                } else if job.stage == .installing {
                    ProgressView().progressViewStyle(.linear).accessibilityLabel("Android 正在安装")
                }
            }
            Spacer(minLength: 4)
            if job.active && job.stage != .installing {
                Button("取消") { model.send(Command(.cancel, jobId: job.id)) }
                    .buttonStyle(.borderless)
            }
            Button {
                present(.job(job.id))
            } label: {
                Label("任务详情", systemImage: "info.circle").labelStyle(.iconOnly)
            }
            .buttonStyle(.borderless)
        }
    }

    private func present(_ sheet: WorkspaceSheet) {
        if let drawerSheet {
            drawerSheet(sheet)
        } else {
            routes.sheet = sheet
        }
    }

    @ViewBuilder private var notice: some View {
        let message = model.actionError ?? model.snapshot.notice
        if !message.isEmpty {
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: model.actionError == nil ? "info.circle" : "exclamationmark.circle")
                Text(message).lineLimit(3).help(message).textSelection(.enabled)
                Spacer(minLength: 4)
                Button {
                    model.send(Command(.clearNotice))
                } label: {
                    Label("关闭提示", systemImage: "xmark").labelStyle(.iconOnly)
                }
                .buttonStyle(.borderless)
            }
            .font(.callout)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(.bar)
        }
    }
}

private struct DropZone: View {
    let model: BackendModel
    let compact: Bool
    @ViewState private var targeting = false

    var body: some View {
        VStack(spacing: 6) {
            Image(systemName: "tray.and.arrow.down")
                .font(.system(size: compact ? 20 : 26))
                .foregroundStyle(.tint)
                .accessibilityHidden(true)
            Text(targeting ? "松开以检查 APK" : "拖放 APK 到此处").font(.headline)
            if !compact {
                Text("支持单包、批量与拆分 APK").font(.callout).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, minHeight: compact ? 72 : 106)
        .background(Color.accentColor.opacity(targeting ? 0.13 : 0.045), in: .rect(cornerRadius: 14))
        .overlay {
            RoundedRectangle(cornerRadius: 14).strokeBorder(
                targeting ? Color.accentColor : Color.secondary.opacity(0.30),
                style: StrokeStyle(lineWidth: 1, dash: targeting ? [] : [5, 4])
            )
            .allowsHitTesting(false)
        }
        .dropDestination(for: URL.self) { files, _ in
            guard files.contains(where: \.isFileURL) else {
                model.actionError = "请拖入 APK 文件。"
                return false
            }
            model.prepare(files, split: false)
            return true
        } isTargeted: { targeting = $0 }
    }
}

@MainActor func pickAPKs(_ model: BackendModel, split: Bool) {
    let panel = NSOpenPanel()
    panel.title = split ? "选择拆分 APK" : "选择 APK"
    panel.allowedContentTypes = [.init(filenameExtension: "apk") ?? .data]
    panel.allowsMultipleSelection = true
    panel.canChooseDirectories = false
    panel.begin { response in
        if response == .OK { model.prepare(panel.urls, split: split) }
    }
}

func byteCount(_ count: UInt64) -> String {
    ByteCountFormatter.string(fromByteCount: Int64(clamping: count), countStyle: .file)
}

private func statusSymbol(_ stage: Job.Stage) -> String {
    switch stage {
    case .transferring: "arrow.down.circle"
    case .installing: "shippingbox"
    case .succeeded: "checkmark.circle.fill"
    case .failed: "exclamationmark.circle.fill"
    case .canceled: "minus.circle"
    case .waiting: "clock"
    }
}

private func statusColor(_ stage: Job.Stage) -> Color {
    switch stage {
    case .succeeded: .green
    case .failed: .red
    case .transferring, .installing: .accentColor
    default: .secondary
    }
}
