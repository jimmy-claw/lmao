import QtQuick 2.15
import QtQuick.Layouts 1.15

Rectangle {
    property string label: ""
    property int value: 0
    property color accent: Theme.palette.primary

    Layout.fillWidth: true
    height: 52
    radius: Theme.spacing.tiny
    color: Theme.palette.background

    ColumnLayout {
        anchors.centerIn: parent
        spacing: 2

        Text {
            Layout.alignment: Qt.AlignHCenter
            text: value
            color: accent
            font { pixelSize: 18; bold: true; family: "monospace" }
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: label
            color: Theme.palette.textTertiary
            font.pixelSize: 10
        }
    }
}
