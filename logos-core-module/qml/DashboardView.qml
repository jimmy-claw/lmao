import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * DashboardView — agent status, identity, topics, and encryption overview.
 *
 * Context properties (injected by LmaoComponent):
 *   - lmaoModule     : LmaoBackend*
 *   - lmaoAgentModel : AgentListModel*
 */
Item {
    id: root

    property var infoData: ({})
    property bool loaded: false
    property string errorMsg: ""

    function refresh() {
        errorMsg = ""
        var result = lmaoModule.getInfo()
        try {
            var obj = JSON.parse(result)
            if (obj.success) {
                infoData = obj
                loaded = true
            } else {
                errorMsg = obj.error || "Unknown error"
            }
        } catch (e) {
            errorMsg = "Failed to parse info response"
        }
    }

    Component.onCompleted: refresh()

    Flickable {
        anchors.fill: parent
        contentHeight: col.implicitHeight + Theme.spacing.large * 2
        clip: true

        ColumnLayout {
            id: col
            anchors {
                left: parent.left; right: parent.right
                top: parent.top
                margins: Theme.spacing.large
            }
            spacing: Theme.spacing.medium

            // ── Status banner ──
            Rectangle {
                Layout.fillWidth: true
                height: 56
                radius: Theme.spacing.radiusLarge
                color: Theme.palette.backgroundSecondary

                RowLayout {
                    anchors { fill: parent; margins: Theme.spacing.medium }
                    spacing: Theme.spacing.medium

                    Rectangle {
                        width: 10; height: 10; radius: 5
                        color: root.loaded ? Theme.palette.success : Theme.palette.error
                    }

                    Text {
                        text: root.loaded ? "Agent Online" : "Initializing…"
                        color: Theme.palette.text
                        font { pixelSize: 15; bold: true; family: "monospace" }
                    }

                    Item { Layout.fillWidth: true }

                    Text {
                        visible: lmaoAgentModel.count > 0
                        text: lmaoAgentModel.count + " peers"
                        color: Theme.palette.textSecondary
                        font.pixelSize: 12
                    }

                    Button {
                        text: "↻"
                        onClicked: root.refresh()
                        width: 32; height: 32
                        contentItem: Text {
                            text: parent.text
                            color: Theme.palette.primary
                            font.pixelSize: 16
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            color: parent.hovered ? Theme.palette.backgroundTertiary : "transparent"
                            radius: Theme.spacing.tiny
                        }
                    }
                }
            }

            // ── Error display ──
            Text {
                visible: root.errorMsg.length > 0
                text: "⚠ " + root.errorMsg
                color: Theme.palette.error
                font.pixelSize: 12
                Layout.fillWidth: true
                wrapMode: Text.WordWrap
            }

            // ── Identity card ──
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: identityCol.implicitHeight + Theme.spacing.large * 2
                radius: Theme.spacing.radiusLarge
                color: Theme.palette.backgroundTertiary
                border { color: Theme.palette.borderSecondary; width: 1 }

                ColumnLayout {
                    id: identityCol
                    anchors {
                        left: parent.left; right: parent.right; top: parent.top
                        margins: Theme.spacing.large
                    }
                    spacing: Theme.spacing.small

                    Text {
                        text: "Identity"
                        color: Theme.palette.text
                        font { pixelSize: 14; bold: true }
                    }

                    InfoRow { label: "Public Key"; value: infoData.public_key || "—"; mono: true; copyable: true }
                    InfoRow { label: "Encryption"; value: infoData.encryption ? "Enabled" : "Disabled"; statusColor: infoData.encryption ? Theme.palette.success : Theme.palette.textTertiary }
                }
            }

            // ── Waku Topics card ──
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: topicsCol.implicitHeight + Theme.spacing.large * 2
                radius: Theme.spacing.radiusLarge
                color: Theme.palette.backgroundTertiary
                border { color: Theme.palette.borderSecondary; width: 1 }

                ColumnLayout {
                    id: topicsCol
                    anchors {
                        left: parent.left; right: parent.right; top: parent.top
                        margins: Theme.spacing.large
                    }
                    spacing: Theme.spacing.small

                    Text {
                        text: "Waku Topics"
                        color: Theme.palette.text
                        font { pixelSize: 14; bold: true }
                    }

                    InfoRow { label: "Task"; value: infoData.task_topic || "—"; mono: true }
                    InfoRow { label: "Discovery"; value: infoData.discovery_topic || "—"; mono: true }
                    InfoRow { label: "Presence"; value: infoData.presence_topic || "—"; mono: true }
                }
            }

            // ── Transport card ──
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: transportCol.implicitHeight + Theme.spacing.large * 2
                radius: Theme.spacing.radiusLarge
                color: Theme.palette.backgroundTertiary
                border { color: Theme.palette.borderSecondary; width: 1 }

                ColumnLayout {
                    id: transportCol
                    anchors {
                        left: parent.left; right: parent.right; top: parent.top
                        margins: Theme.spacing.large
                    }
                    spacing: Theme.spacing.small

                    Text {
                        text: "Transport"
                        color: Theme.palette.text
                        font { pixelSize: 14; bold: true }
                    }

                    InfoRow {
                        label: "QtRO Delivery"
                        value: lmaoModule.hasDeliveryTransport ? "Connected" : "Unavailable"
                        statusColor: lmaoModule.hasDeliveryTransport ? Theme.palette.success : Theme.palette.textTertiary
                    }
                }
            }
        }
    }

    // ── InfoRow helper component ──
    component InfoRow: RowLayout {
        property string label: ""
        property string value: ""
        property bool mono: false
        property bool copyable: false
        property color statusColor: "transparent"

        Layout.fillWidth: true
        spacing: Theme.spacing.small

        Text {
            text: label
            color: Theme.palette.textSecondary
            font.pixelSize: 12
            Layout.preferredWidth: 100
        }

        Rectangle {
            visible: statusColor != "transparent"
            width: 8; height: 8; radius: 4
            color: statusColor
        }

        Text {
            text: {
                if (!mono) return value
                if (value.length > 48)
                    return value.substring(0, 20) + "…" + value.slice(-12)
                return value
            }
            color: Theme.palette.text
            font { pixelSize: 12; family: mono ? "monospace" : "sans-serif" }
            Layout.fillWidth: true
            elide: Text.ElideRight

            MouseArea {
                anchors.fill: parent
                visible: copyable
                cursorShape: copyable ? Qt.PointingHandCursor : Qt.ArrowCursor
                onClicked: {
                    // Copy full value to clipboard
                    clipHelper.text = value
                    clipHelper.selectAll()
                    clipHelper.copy()
                    copiedTip.visible = true
                    copiedTimer.restart()
                }
            }
        }

        Text {
            id: copiedTip
            visible: false
            text: "Copied!"
            color: Theme.palette.success
            font.pixelSize: 10

            Timer {
                id: copiedTimer
                interval: 1500
                onTriggered: copiedTip.visible = false
            }
        }
    }

    // Hidden TextInput for clipboard support
    TextInput {
        id: clipHelper
        visible: false
    }
}
