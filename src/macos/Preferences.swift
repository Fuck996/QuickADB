// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import AppKit
import ServiceManagement
import SwiftUI

enum Appearance: String, CaseIterable {
    case system = "跟随系统"
    case light = "浅色"
    case dark = "深色"

    @MainActor func apply() {
        switch self {
        case .system: NSApp.appearance = nil
        case .light: NSApp.appearance = NSAppearance(named: .aqua)
        case .dark: NSApp.appearance = NSAppearance(named: .darkAqua)
        }
    }
}

struct Preferences: View {
    let model: BackendModel
    @AppStorage("appearance") private var appearance = Appearance.system
    @State private var loginStatus = SMAppService.mainApp.status
    @State private var loginError: String?

    var body: some View {
        Form {
            Section("外观") {
                Picker("主题", selection: $appearance) {
                    ForEach(Appearance.allCases, id: \.self) { item in
                        Text(item.rawValue).tag(item)
                    }
                }
                .onChange(of: appearance) { appearance.apply() }
            }
            Section("常驻") {
                Toggle("登录时启动到菜单栏", isOn: Binding(
                    get: { loginStatus == .enabled || loginStatus == .requiresApproval },
                    set: setLaunchAtLogin
                ))
                if loginStatus == .requiresApproval {
                    Button("打开系统设置，允许登录项") { SMAppService.openSystemSettingsLoginItems() }
                }
                if let loginError {
                    Text(loginError).foregroundStyle(.red).textSelection(.enabled)
                }
            }
            Section("安装") {
                Toggle("允许安装标记为仅测试的 APK", isOn: Binding(
                    get: { model.snapshot.testPackages },
                    set: { model.send(Command(.preferences, testPackages: $0)) }
                ))
                if let error = model.actionError {
                    Text(error).foregroundStyle(.red).textSelection(.enabled)
                }
            }
            Section("支持") {
                LabeledContent("日志和设置") {
                    Button("在 Finder 中显示") { NSWorkspace.shared.open(model.directory) }
                }
                LabeledContent("许可") {
                    Menu("查看") {
                        resourceButton("QuickADB 来源声明", file: "NOTICE")
                        resourceButton("GPL-3.0 许可全文", file: "LICENSE")
                        resourceButton("第三方许可", file: "ThirdPartyNotices.txt")
                    }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 460, height: 410)
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            loginStatus = SMAppService.mainApp.status
        }
    }

    private func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
            loginError = nil
        } catch {
            loginError = "无法更改登录项：\(error.localizedDescription)"
        }
        loginStatus = SMAppService.mainApp.status
    }

    private func resourceButton(_ label: String, file: String) -> some View {
        Button(label) {
            if let url = Bundle.main.resourceURL?.appending(path: file) {
                NSWorkspace.shared.open(url)
            }
        }
    }
}
