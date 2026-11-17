// Copyright (C) 2026 QuickADB contributors
// SPDX-License-Identifier: GPL-3.0-only

import AppKit

enum StatusIcon {
    static func adbRobot() -> NSImage {
        let image = NSImage(size: NSSize(width: 20, height: 18), flipped: false) { _ in
            NSColor.black.setFill()

            let antenna = NSBezierPath(roundedRect: NSRect(x: 9.25, y: 11, width: 1.5, height: 4.2),
                                       xRadius: 0.75, yRadius: 0.75)
            antenna.appendOval(in: NSRect(x: 8.45, y: 13.8, width: 3.1, height: 3.1))
            antenna.fill()

            for x in [1.4, 15.8] {
                NSBezierPath(roundedRect: NSRect(x: x, y: 5.3, width: 2.8, height: 4.8),
                             xRadius: 1.4, yRadius: 1.4).fill()
            }

            let face = NSBezierPath(roundedRect: NSRect(x: 3.4, y: 2.6, width: 13.2, height: 10.5),
                                    xRadius: 3.7, yRadius: 3.7)
            face.windingRule = .evenOdd
            for x in [6.35, 11.35] {
                face.appendRoundedRect(NSRect(x: x, y: 6.1, width: 2.3, height: 3.1),
                                       xRadius: 1.15, yRadius: 1.15)
            }
            face.fill()
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = "QuickADB 安装抽屉"
        return image
    }
}
