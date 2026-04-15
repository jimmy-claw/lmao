import QtQuick 2.15
import QtQuick.Layouts 1.15

RowLayout {
    property string label: ""
    property string value: ""
    property bool mono: false

    spacing: Theme.spacing.small

    Text {
        text: label
        color: Theme.palette.textSecondary
        font.pixelSize: 12
        Layout.preferredWidth: 100
    }
    Text {
        text: value
        color: Theme.palette.text
        font { pixelSize: 12; family: mono ? "monospace" : "" }
        Layout.fillWidth: true
        elide: Text.ElideRight
    }
}
