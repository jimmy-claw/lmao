pragma Singleton
import QtQuick 2.15

QtObject {
    id: theme

    readonly property QtObject palette: QtObject {
        readonly property color background:          "#171717"
        readonly property color backgroundSecondary: "#262626"
        readonly property color backgroundTertiary:  "#1C1C1C"
        readonly property color borderSecondary:     "#2B303B"
        readonly property color primary:             "#2196F3"
        readonly property color primaryHover:        "#1976D2"
        readonly property color text:                "#FFFFFF"
        readonly property color textSecondary:       "#A4A4A4"
        readonly property color textTertiary:        "#969696"
        readonly property color success:             "#4CAF50"
        readonly property color error:               "#F44336"
    }

    readonly property QtObject spacing: QtObject {
        readonly property int tiny:   4
        readonly property int small:  8
        readonly property int medium: 12
        readonly property int large:  16
        readonly property int xlarge: 20
        readonly property int radiusLarge:  8
        readonly property int radiusXlarge: 16
    }
}
