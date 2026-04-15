import QtQuick 2.15

Rectangle {
    id: root

    property string label: ""
    property int value: 0
    property color valueColor: Theme.textPrimary

    width: 140
    height: 72
    radius: Theme.radiusLarge
    color: Theme.surface
    border.color: Theme.border
    border.width: 1

    Column {
        anchors.centerIn: parent
        spacing: Theme.tiny

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: root.value
            font.pixelSize: 22
            font.bold: true
            color: root.valueColor
        }

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: root.label
            font.pixelSize: 11
            color: Theme.textSecondary
        }
    }
}
