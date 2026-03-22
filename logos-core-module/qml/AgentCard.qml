import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * AgentCard — delegate for a single discovered A2A agent.
 */
Rectangle {
    id: root

    property string agentName:    ""
    property string agentDesc:    ""
    property string agentPubkey:  ""
    property string agentVersion: ""

    signal sendTaskRequested(string pubkey)

    height: cardColumn.implicitHeight + 24
    radius: Theme.spacing.radiusLarge
    color:  mouseArea.containsMouse ? Theme.palette.backgroundSecondary : Theme.palette.backgroundTertiary

    border {
        color: mouseArea.containsMouse ? Theme.palette.primary : Theme.palette.borderSecondary
        width: 1
    }

    Behavior on color        { ColorAnimation { duration: 120 } }
    Behavior on border.color { ColorAnimation { duration: 120 } }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: root.sendTaskRequested(root.agentPubkey)
    }

    ColumnLayout {
        id: cardColumn
        anchors {
            left: parent.left; right: parent.right; top: parent.top
            margins: Theme.spacing.medium
        }
        spacing: Theme.spacing.tiny

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing.small

            Text {
                text: root.agentName || "(unnamed)"
                color: Theme.palette.text
                font { pixelSize: 15; bold: true; family: "monospace" }
                elide: Text.ElideRight
                Layout.fillWidth: true
            }

            Text {
                text: "v" + (root.agentVersion || "?")
                color: Theme.palette.textSecondary
                font.pixelSize: 12
            }
        }

        Text {
            visible: root.agentDesc.length > 0
            text: root.agentDesc.length > 120
                  ? root.agentDesc.substring(0, 120) + "\u2026"
                  : root.agentDesc
            color: Theme.palette.textSecondary
            font.pixelSize: 12
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Text {
            text: root.agentPubkey.length > 16
                  ? root.agentPubkey.substring(0, 8) + "\u2026" + root.agentPubkey.slice(-6)
                  : root.agentPubkey
            color: Theme.palette.primary
            font { pixelSize: 11; family: "monospace" }
        }
    }
}
