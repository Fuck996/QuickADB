// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import SwiftUI

struct ConnectionSheet: View {
    private enum Mode: String, CaseIterable {
        case pair = "无线配对"
        case paired = "已配对设备"
        case tcp = "传统 TCP"
    }

    let model: BackendModel
    @Environment(\.dismiss) private var dismiss
    @State private var mode = Mode.pair
    @State private var host = ""
    @State private var port = ""
    @State private var code = ""
    @State private var pairingId: String?
    @State private var pairedId = ""
    @State private var manual = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("连接设备").font(.title2.weight(.semibold))
            Picker("连接方式", selection: Binding(get: { mode }, set: {
                mode = $0
                resetAddress()
            })) {
                ForEach(Mode.allCases, id: \.self) { mode in
                    Text(mode.rawValue).tag(mode)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .disabled(busy)

            VStack(alignment: .leading, spacing: 5) {
                Text("局域网发现").font(.headline)
                if visibleServices.isEmpty {
                    Text(mode == .pair
                         ? "正在查找可配对设备。请保持手机的配对码弹窗开启，并允许 QuickADB 访问本地网络。"
                         : "暂未发现设备。请开启手机的无线调试，并确认 QuickADB 获准访问本地网络。")
                        .font(.callout).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    List(visibleServices) { service in
                        Button { choose(service) } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(service.name).fontWeight(.medium).lineLimit(1)
                                    Text("\(service.kind.title) · \(service.endpoint.address)")
                                        .font(.caption).foregroundStyle(.secondary)
                                }
                                Spacer()
                                if mode == .pair {
                                    if service.id == pairingService?.id && !manual {
                                        Image(systemName: "checkmark").foregroundStyle(Color.accentColor)
                                    }
                                } else {
                                    Image(systemName: "chevron.forward").foregroundStyle(.secondary)
                                }
                            }
                            .contentShape(.rect)
                        }
                        .buttonStyle(.plain)
                        .disabled(busy)
                        .padding(.vertical, 3)
                    }
                    .listStyle(.bordered)
                    .frame(height: 130)
                }
            }

            Group {
                switch mode {
                case .pair:
                    Text("在手机的无线调试设置中打开「使用配对码配对设备」。发现多台设备时，请先选择你的手机。")
                        .font(.callout).foregroundStyle(.secondary)
                    DisclosureGroup("手动输入 IP 地址和配对端口", isExpanded: Binding(
                        get: { manual }, set: { manual = $0; code = "" }
                    )) {
                        addressFields(label: "配对端口").padding(.top, 6)
                    }
                    SecureField("六位配对码", text: Binding(get: { code }, set: { value in
                        if !manual, let service = pairingService {
                            pairingId = service.id
                        }
                        code = value
                    }))
                    .textFieldStyle(.roundedBorder)
                    .disabled(!manual && pairingService == nil)
                case .paired:
                    let paired = model.snapshot.devices.filter { $0.pairedId != nil }
                    if paired.isEmpty {
                        Text("暂无已配对设备").foregroundStyle(.secondary)
                    } else {
                        Picker("设备", selection: $pairedId) {
                            Text("选择设备").tag("")
                            ForEach(paired) { device in
                                if let id = device.pairedId {
                                    Text(device.name).tag(id)
                                }
                            }
                        }
                        Text("默认通过局域网发现当前连接端口。").font(.callout).foregroundStyle(.secondary)
                        DisclosureGroup("手动指定连接地址", isExpanded: $manual) {
                            addressFields(label: "连接端口").padding(.top, 6)
                        }
                    }
                case .tcp:
                    Text("输入已开启传统 ADB TCP 调试的设备地址。首次连接可能需要在手机上授权。")
                        .font(.callout).foregroundStyle(.secondary)
                    addressFields(label: "连接端口")
                }
            }
            .disabled(busy)

            SheetFeedback(model: model)
            HStack {
                if busy {
                    ProgressView().controlSize(.small)
                    Text(model.snapshot.pairing ? "正在配对…" : "正在连接…").foregroundStyle(.secondary)
                }
                Spacer()
                Button("关闭") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(mode == .pair ? "配对" : "连接", action: submit)
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
                    .disabled(!canSubmit)
            }
        }
        .padding(22)
        .frame(width: 480)
    }

    private var busy: Bool { model.snapshot.pairing || model.snapshot.connecting }
    private var addressValid: Bool { !host.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && (UInt16(port) ?? 0) > 0 }

    private var pairingServices: [Service] { model.snapshot.discovered.filter { $0.kind == .pairing } }

    private var visibleServices: [Service] {
        mode == .pair ? pairingServices : model.snapshot.discovered
    }

    private var pairingService: Service? {
        if let pairingId {
            return pairingServices.first { $0.id == pairingId }
        }
        return pairingServices.count == 1 ? pairingServices.first : nil
    }

    private var canSubmit: Bool {
        guard !busy else { return false }
        switch mode {
        case .pair:
            return (manual ? addressValid : pairingService != nil)
                && code.utf8.count == 6 && code.utf8.allSatisfy { (48...57).contains($0) }
        case .paired: return !pairedId.isEmpty && (!manual || addressValid)
        case .tcp: return addressValid
        }
    }

    private func addressFields(label: String) -> some View {
        HStack {
            TextField("手机 IP 地址", text: $host)
            TextField(label, text: $port).frame(width: 110)
        }
        .textFieldStyle(.roundedBorder)
    }

    private func resetAddress() {
        host = ""
        port = ""
        code = ""
        pairingId = nil
        pairedId = ""
        manual = false
    }

    private func choose(_ service: Service) {
        let newMode: Mode
        switch service.kind {
        case .pairing: newMode = .pair
        case .tls: newMode = service.pairedId == nil ? .pair : .paired
        case .tcp: newMode = .tcp
        }
        if service.kind == .pairing, mode == .pair, !manual, pairingService?.id == service.id {
            return
        }
        mode = newMode
        resetAddress()
        if service.kind == .pairing {
            pairingId = service.id
            return
        }
        host = service.endpoint.host
        pairedId = service.pairedId ?? ""
        if service.kind == .tcp {
            port = String(service.endpoint.port)
        }
    }

    private func submit() {
        let command: Command
        switch mode {
        case .pair:
            if manual {
                command = Command(.pair, host: host.trimmingCharacters(in: .whitespacesAndNewlines),
                                  port: UInt16(port), code: code)
            } else {
                guard let endpoint = pairingService?.endpoint else { return }
                command = Command(.pair, host: endpoint.host, port: endpoint.port, code: code)
            }
        case .paired:
            command = Command(.connectPaired, id: pairedId, host: manual ? host : nil,
                              port: manual ? UInt16(port) : nil)
        case .tcp:
            command = Command(.connect, host: host, port: UInt16(port))
        }
        if model.send(command), mode == .pair { code = "" }
    }
}

struct DeviceSheet: View {
    let model: BackendModel
    let id: String
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var host = ""
    @State private var port = ""
    @State private var expanded = false
    @State private var automatic = true
    @State private var forget = false

    private var device: Device? { model.snapshot.devices.first { $0.id == id } }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            if let device {
                Text(device.name).font(.title2.weight(.semibold)).textSelection(.enabled)
                Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 6) {
                    detail("连接", "\(device.transport) · \(device.status)")
                    detail("型号", device.model)
                    detail("序列号", device.serial)
                    detail("Android", device.android)
                    if let endpoint = device.endpoint { detail("地址", endpoint.address) }
                }
                if !device.detail.isEmpty {
                    ScrollView {
                        Text(device.detail).textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 95)
                }
                HStack {
                    TextField("设备备注", text: $name).textFieldStyle(.roundedBorder)
                    Button("保存备注") { model.send(Command(.alias, id: id, name: name)) }
                        .disabled((device.alias ?? "") == name)
                }
                DisclosureGroup(device.wireless ? "编辑连接地址" : "切换为传统 TCP 调试",
                                isExpanded: $expanded) {
                    VStack(alignment: .leading, spacing: 8) {
                        if device.pairedId != nil {
                            Toggle("通过局域网自动获取连接端口", isOn: $automatic)
                        }
                        HStack {
                            TextField("手机 IP 地址", text: $host)
                            TextField("连接端口", text: $port).frame(width: 100)
                        }
                        .textFieldStyle(.roundedBorder)
                        .disabled(device.pairedId != nil && automatic)
                        Button(device.wireless ? "保存并连接" : "切换并连接") {
                            let useDiscovery = device.pairedId != nil && automatic
                            model.send(Command(device.wireless ? .editConnection : .enableTcp,
                                               id: id, host: useDiscovery ? nil : host,
                                               port: useDiscovery ? nil : UInt16(port),
                                               pairedId: device.pairedId))
                        }
                        .disabled(!canSave(device))
                    }
                    .padding(.top, 8)
                }
                .disabled(device.managing || (!device.wireless && !device.online))

                SheetFeedback(model: model)
                HStack {
                    if device.wireless {
                        Button("删除记录…", role: .destructive) { forget = true }
                            .disabled(device.managing)
                        if device.online {
                            Button("断开") { model.send(Command(.disconnect, id: id)) }
                                .disabled(device.managing)
                        }
                    }
                    if !device.online {
                        Button("重新连接") { model.send(Command(.retryConnection, id: id)) }
                            .disabled(device.managing || model.snapshot.connecting)
                    }
                    Spacer()
                    Button("关闭") { dismiss() }.keyboardShortcut(.cancelAction)
                }
            } else {
                Text("设备记录已不存在").foregroundStyle(.secondary)
                SheetFeedback(model: model)
                Button("关闭") { dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(22)
        .frame(width: 465)
        .onAppear {
            name = device?.alias ?? ""
            host = device?.endpoint?.host ?? ""
            port = device?.endpoint.map { String($0.port) } ?? (device?.wireless == false ? "5555" : "")
        }
        .alert("删除此设备记录？", isPresented: $forget) {
            Button("取消", role: .cancel) {}
            Button("删除", role: .destructive) { model.send(Command(.forget, id: id)) }
        } message: {
            Text("将删除此电脑保存的连接、配对记录和备注。")
        }
    }

    @ViewBuilder private func detail(_ title: String, _ value: String) -> some View {
        if !value.isEmpty {
            GridRow {
                Text(title).foregroundStyle(.secondary)
                Text(value).textSelection(.enabled)
            }
        }
    }

    private func canSave(_ device: Device) -> Bool {
        (device.pairedId != nil && automatic)
            || (!host.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && (UInt16(port) ?? 0) > 0)
    }
}

struct JobSheet: View {
    let model: BackendModel
    let id: UInt64
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let job = model.snapshot.jobs.first(where: { $0.id == id }) {
                Text(job.title).font(.title2.weight(.semibold)).textSelection(.enabled)
                Text("\(job.device) · \(job.status)").foregroundStyle(.secondary)
                if !job.detail.isEmpty {
                    ScrollView {
                        Text(job.detail).textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(maxHeight: 170)
                }
                SheetFeedback(model: model)
                HStack {
                    if [.failed, .canceled].contains(job.stage) {
                        Button("重试", action: { model.send(Command(.retryJob, jobId: id)) })
                    }
                    Spacer()
                    Button("关闭") { dismiss() }.keyboardShortcut(.cancelAction)
                }
            } else {
                Text("任务已清理").foregroundStyle(.secondary)
                Button("关闭") { dismiss() }.keyboardShortcut(.cancelAction)
            }
        }
        .padding(22)
        .frame(width: 450)
    }
}

private struct SheetFeedback: View {
    let model: BackendModel

    var body: some View {
        let message = model.actionError ?? model.snapshot.notice
        if !message.isEmpty {
            Text(message)
                .font(.callout)
                .foregroundStyle(model.actionError == nil ? Color.secondary : Color.red)
                .textSelection(.enabled)
        }
    }
}
