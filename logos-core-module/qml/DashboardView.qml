import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * DashboardView — Agent identity, status, topics, and peer overview.
 *
 * Context properties (injected by LmaoComponent):
 *   - lmaoModule : LmaoBackend*
 */
Item {
    id: dashRoot

    property var nodeInfo: ({})
    property var peers: []
    property bool loaded: false

    function refresh() {
        var infoRaw = lmaoModule.getNodeInfo()
        try {
            var infoObj = JSON.parse(infoRaw)
            if (infoObj.success) nodeInfo = infoObj.info
        } catch(e) {}

        var peersRaw = lmaoModule.getPeers()
        try {
            var peersObj = JSON.parse(peersRaw)
            if (peersObj.success) peers = peersObj.peers
        } catch(e) {}

        loaded = true
    }

    Component.onCompleted: refresh()

    Flickable {
        anchors.fill: parent
        anchors.margins: Theme.spacing.medium
        contentHeight: col.implicitHeight
        clip: true

        ColumnLayout {
            id: col
            width: parent.width
            spacing: Theme.spacing.medium

            // ── Agent Identity ──────────────────────────────────────────
            Rectangle {
                Layout.fillWidth: true
                height: identityCol.implicitHeight + Theme.spacing.large * 2
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
                        text: "Agent Identity"
                        color: Theme.palette.primary
                        font { pixelSize: 14; bold: true }
                    }

                    // Status row
                    RowLayout {
                        spacing: Theme.spacing.small
                        Rectangle {
                            width: 8; height: 8; radius: 4
                            color: loaded ? Theme.palette.success : Theme.palette.error
                        }
                        Text {
                            text: loaded ? "Online" : "Offline"
                            color: loaded ? Theme.palette.success : Theme.palette.error
                            font.pixelSize: 12
                        }
                        Item { Layout.fillWidth: true }
                        Text {
                            text: "v" + (nodeInfo.version || "—")
                            color: Theme.palette.textTertiary
                            font.pixelSize: 11
                        }
                    }

                    InfoRow { label: "Name";        value: nodeInfo.name || "—" }
                    InfoRow { label: "Description";  value: nodeInfo.description || "—" }

                    // Public key (truncated, monospace)
                    RowLayout {
                        spacing: Theme.spacing.small
                        Text {
                            text: "Public Key"
                            color: Theme.palette.textSecondary
                            font.pixelSize: 12
                            Layout.preferredWidth: 100
                        }
                        Text {
                            property string pk: nodeInfo.public_key || ""
                            text: pk.length > 16 ? pk.substring(0, 8) + "..." + pk.substring(pk.length - 6) : pk
                            color: Theme.palette.text
                            font { pixelSize: 12; family: "monospace" }

                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    copyHelper.text = parent.pk
                                    copyHelper.selectAll()
                                    copyHelper.copy()
                                    copyToast.visible = true
                                    copyTimer.restart()
                                }
                            }
                        }
                        Text {
                            text: "(click to copy)"
                            color: Theme.palette.textTertiary
                            font.pixelSize: 10
                        }
                    }

                    InfoRow { label: "Encryption";   value: nodeInfo.encrypted ? "Enabled" : "Disabled" }
                    InfoRow { label: "Capabilities";  value: (nodeInfo.capabilities || []).join(", ") || "—" }
                }
            }

            // ── Waku Topics ─────────────────────────────────────────────
            Rectangle {
                Layout.fillWidth: true
                height: topicsCol.implicitHeight + Theme.spacing.large * 2
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
                        color: Theme.palette.primary
                        font { pixelSize: 14; bold: true }
                    }

                    InfoRow { label: "Task";      value: (nodeInfo.topics || {}).task || "—"; mono: true }
                    InfoRow { label: "Discovery"; value: (nodeInfo.topics || {}).discovery || "—"; mono: true }
                    InfoRow { label: "Presence";  value: (nodeInfo.topics || {}).presence || "—"; mono: true }
                }
            }

            // ── Connected Peers ─────────────────────────────────────────
            Rectangle {
                Layout.fillWidth: true
                height: peersCol.implicitHeight + Theme.spacing.large * 2
                radius: Theme.spacing.radiusLarge
                color: Theme.palette.backgroundTertiary
                border { color: Theme.palette.borderSecondary; width: 1 }

                ColumnLayout {
                    id: peersCol
                    anchors {
                        left: parent.left; right: parent.right; top: parent.top
                        margins: Theme.spacing.large
                    }
                    spacing: Theme.spacing.small

                    RowLayout {
                        Text {
                            text: "Connected Peers"
                            color: Theme.palette.primary
                            font { pixelSize: 14; bold: true }
                        }
                        Item { Layout.fillWidth: true }
                        Rectangle {
                            width: peerCountText.implicitWidth + Theme.spacing.medium
                            height: 20; radius: 10
                            color: Theme.palette.backgroundSecondary
                            Text {
                                id: peerCountText
                                anchors.centerIn: parent
                                text: peers.length
                                color: Theme.palette.primary
                                font { pixelSize: 12; bold: true }
                            }
                        }
                    }

                    Repeater {
                        model: peers
                        delegate: Rectangle {
                            Layout.fillWidth: true
                            height: peerRow.implicitHeight + Theme.spacing.small * 2
                            radius: Theme.spacing.tiny
                            color: Theme.palette.background

                            RowLayout {
                                id: peerRow
                                anchors {
                                    left: parent.left; right: parent.right
                                    verticalCenter: parent.verticalCenter
                                    margins: Theme.spacing.small
                                }
                                spacing: Theme.spacing.small

                                Rectangle {
                                    width: 6; height: 6; radius: 3
                                    color: Theme.palette.success
                                }
                                Text {
                                    text: modelData.name || "unknown"
                                    color: Theme.palette.text
                                    font { pixelSize: 12; bold: true }
                                }
                                Text {
                                    text: (modelData.capabilities || []).join(", ")
                                    color: Theme.palette.textTertiary
                                    font.pixelSize: 11
                                    Layout.fillWidth: true
                                    elide: Text.ElideRight
                                }
                            }
                        }
                    }

                    Text {
                        visible: peers.length === 0
                        text: "No live peers"
                        color: Theme.palette.textTertiary
                        font.pixelSize: 12
                    }
                }
            }

            // ── Sessions ────────────────────────────────────────────────
            Rectangle {
                Layout.fillWidth: true
                height: sessionsCol.implicitHeight + Theme.spacing.large * 2
                radius: Theme.spacing.radiusLarge
                color: Theme.palette.backgroundTertiary
                border { color: Theme.palette.borderSecondary; width: 1 }

                ColumnLayout {
                    id: sessionsCol
                    anchors {
                        left: parent.left; right: parent.right; top: parent.top
                        margins: Theme.spacing.large
                    }
                    spacing: Theme.spacing.small

                    RowLayout {
                        Text {
                            text: "Active Sessions"
                            color: Theme.palette.primary
                            font { pixelSize: 14; bold: true }
                        }
                        Item { Layout.fillWidth: true }
                        Text {
                            text: (nodeInfo.sessions_count || 0) + " sessions"
                            color: Theme.palette.textSecondary
                            font.pixelSize: 11
                        }
                    }
                }
            }

            // Refresh button
            Button {
                Layout.alignment: Qt.AlignHCenter
                text: "Refresh"
                onClicked: dashRoot.refresh()
                contentItem: Text {
                    text: parent.text
                    color: Theme.palette.primary
                    font.pixelSize: 13
                    horizontalAlignment: Text.AlignHCenter
                }
                background: Rectangle {
                    color: parent.hovered ? Theme.palette.backgroundSecondary : "transparent"
                    radius: Theme.spacing.tiny
                    border { color: Theme.palette.primary; width: 1 }
                }
                height: 30
            }
        }
    }

    // Hidden text input for clipboard copy
    TextInput {
        id: copyHelper
        visible: false
    }

    // Copy toast
    Rectangle {
        id: copyToast
        visible: false
        anchors.bottom: parent.bottom
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.bottomMargin: Theme.spacing.large
        width: toastText.implicitWidth + Theme.spacing.large * 2
        height: 28; radius: 14
        color: Theme.palette.success

        Text {
            id: toastText
            anchors.centerIn: parent
            text: "Copied to clipboard"
            color: "#FFFFFF"
            font.pixelSize: 11
        }

        Timer {
            id: copyTimer
            interval: 2000
            onTriggered: copyToast.visible = false
        }
    }
}
