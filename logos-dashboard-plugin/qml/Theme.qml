pragma Singleton
import QtQuick 2.15

QtObject {
    // ── Palette ──
    readonly property color background:      "#171717"
    readonly property color surface:         "#1e1e1e"
    readonly property color surfaceHover:    "#2a2a2a"
    readonly property color border:          "#333333"

    readonly property color primary:         "#2196F3"
    readonly property color primaryDim:      "#1565C0"
    readonly property color accent:          "#4FC3F7"
    readonly property color success:         "#4CAF50"
    readonly property color warning:         "#FF9800"
    readonly property color error:           "#F44336"

    readonly property color textPrimary:     "#E0E0E0"
    readonly property color textSecondary:   "#9E9E9E"
    readonly property color textTertiary:    "#616161"

    // ── Spacing ──
    readonly property int tiny:   4
    readonly property int small:  8
    readonly property int medium: 12
    readonly property int large:  16
    readonly property int xlarge: 24

    readonly property int radiusSmall:  4
    readonly property int radiusLarge:  8
}
