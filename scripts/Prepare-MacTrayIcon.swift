// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import AppKit

let input = URL(fileURLWithPath: CommandLine.arguments[1])
let output = URL(fileURLWithPath: CommandLine.arguments[2])
guard let source = NSBitmapImageRep(data: try Data(contentsOf: input)),
      source.hasAlpha, let original = source.cgImage else {
    fatalError("菜单栏图标需要包含透明通道的位图")
}

var left = source.pixelsWide
var right = -1
var top = source.pixelsHigh
var bottom = -1
for y in 0..<source.pixelsHigh {
    for x in 0..<source.pixelsWide where (source.colorAt(x: x, y: y)?.alphaComponent ?? 0) > 0 {
        left = min(left, x)
        right = max(right, x)
        top = min(top, y)
        bottom = max(bottom, y)
    }
}
guard right >= left, bottom >= top,
      let cropped = original.cropping(to: CGRect(x: left, y: top, width: right - left + 1, height: bottom - top + 1)),
      let png = NSBitmapImageRep(cgImage: cropped).representation(using: .png, properties: [:]) else {
    fatalError("无法裁切菜单栏图标")
}
try png.write(to: output)
