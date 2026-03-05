import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * LmaoView — basic fleet view for A2A agent discovery and task sending.
 *
 * Context properties (injected by LmaoComponent):
 *   - lmaoAgentModel : AgentListModel*
 *   - lmaoModule     : LmaoBackend*
 */
Item {
    id: root

    property bool discovering: false
    property string lastError: ""
    property string lastResult: ""

    Rectangle {
        anchors.fill: parent
        color: Theme.palette.background
    }

    // ── Header ─────────────────────────────────────────────────────────
    Rectangle {
        id: header
        anchors { left: parent.left; right: parent.right; top: parent.top }
        height: 56
        color: Theme.palette.backgroundSecondary

        RowLayout {
            anchors { fill: parent; leftMargin: Theme.spacing.large; rightMargin: Theme.spacing.large }
            spacing: 10

            Text {
                text: "LMAO Fleet"
                color: Theme.palette.primary
                font { pixelSize: 18; bold: true; family: "monospace" }
            }

            // ── Status indicator ──
            Rectangle {
                width: 8; height: 8; radius: 4
                color: Theme.palette.success
            }
            Text {
                text: "Online"
                color: Theme.palette.success
                font.pixelSize: 11
            }

            // ── Peer count ──
            Rectangle {
                visible: lmaoAgentModel.count > 0
                width: countText.implicitWidth + Theme.spacing.medium
                height: 20
                radius: 10
                color: Theme.palette.backgroundSecondary

                Text {
                    id: countText
                    anchors.centerIn: parent
                    text: lmaoAgentModel.count + " peers"
                    color: Theme.palette.textSecondary
                    font.pixelSize: 11
                }
            }

            Item { Layout.fillWidth: true }

            BusyIndicator {
                running: root.discovering
                width: 20; height: 20
                visible: root.discovering
            }

            Button {
                text: "Discover"
                enabled: !root.discovering
                onClicked: {
                    root.discovering = true
                    root.lastError = ""
                    var result = lmaoModule.discoverAgents("5000")
                    lmaoAgentModel.loadFromJson(result)
                    root.discovering = false
                }
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

    // ── Body ────────────────────────────────────────────────────────────
    ColumnLayout {
        anchors {
            left: parent.left; right: parent.right
            top: header.bottom; bottom: parent.bottom
            margins: Theme.spacing.medium
        }
        spacing: Theme.spacing.medium

        // ── Agent List ──────────────────────────────────────────────────
        ListView {
            id: agentList
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.spacing.small
            clip: true
            model: lmaoAgentModel

            ScrollBar.vertical: ScrollBar {}

            delegate: AgentCard {
                width: agentList.width
                agentName:    model.name       || ""
                agentDesc:    model.description || ""
                agentPubkey:  model.pubkey      || ""
                agentVersion: model.version     || ""

                onSendTaskRequested: function(pubkey) {
                    taskPubkeyField.text = pubkey
                }
            }

            // Empty state
            Column {
                anchors.centerIn: parent
                spacing: Theme.spacing.medium
                visible: lmaoAgentModel.count === 0 && !root.discovering

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "No agents discovered"
                    color: Theme.palette.textTertiary
                    font.pixelSize: 15
                }
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "Click Discover to search the Waku network"
                    color: Theme.palette.textTertiary
                    font.pixelSize: 12
                }
            }
        }

        // ── Send Task Form ──────────────────────────────────────────────
        Rectangle {
            Layout.fillWidth: true
            height: sendColumn.implicitHeight + Theme.spacing.large * 2
            radius: Theme.spacing.radiusLarge
            color: Theme.palette.backgroundTertiary
            border { color: Theme.palette.borderSecondary; width: 1 }

            ColumnLayout {
                id: sendColumn
                anchors {
                    left: parent.left; right: parent.right
                    top: parent.top
                    margins: Theme.spacing.large
                }
                spacing: Theme.spacing.small

                Text {
                    text: "Send Test Task"
                    color: Theme.palette.text
                    font { pixelSize: 14; bold: true }
                }

                TextField {
                    id: taskPubkeyField
                    Layout.fillWidth: true
                    placeholderText: "Agent public key"
                    color: Theme.palette.text
                    font.pixelSize: 12
                    background: Rectangle {
                        color: Theme.palette.background
                        border { color: Theme.palette.borderSecondary; width: 1 }
                        radius: Theme.spacing.tiny
                    }
                }

                TextField {
                    id: taskTextField
                    Layout.fillWidth: true
                    placeholderText: "Task text"
                    color: Theme.palette.text
                    font.pixelSize: 12
                    background: Rectangle {
                        color: Theme.palette.background
                        border { color: Theme.palette.borderSecondary; width: 1 }
                        radius: Theme.spacing.tiny
                    }
                }

                RowLayout {
                    spacing: Theme.spacing.small

                    Button {
                        text: "Send"
                        enabled: taskPubkeyField.text.length > 0 && taskTextField.text.length > 0
                        onClicked: {
                            var result = lmaoModule.sendTask(taskPubkeyField.text, taskTextField.text)
                            root.lastResult = result
                            taskTextField.text = ""
                        }
                        contentItem: Text {
                            text: parent.text
                            color: parent.enabled ? "#FFFFFF" : Theme.palette.textTertiary
                            font.pixelSize: 13
                            horizontalAlignment: Text.AlignHCenter
                        }
                        background: Rectangle {
                            color: parent.enabled ? Theme.palette.primary : Theme.palette.backgroundSecondary
                            radius: Theme.spacing.tiny
                        }
                        height: 32
                        width: 80
                    }

                    Text {
                        visible: root.lastResult.length > 0
                        text: {
                            if (root.lastResult.length > 60)
                                return root.lastResult.substring(0, 60) + "\u2026"
                            return root.lastResult
                        }
                        color: Theme.palette.textSecondary
                        font.pixelSize: 11
                        Layout.fillWidth: true
                        elide: Text.ElideRight
                    }
                }
            }
        }

        // ── Last task info ──
        Text {
            visible: root.lastResult.length > 0
            text: "Last result: " + root.lastResult
            color: Theme.palette.textTertiary
            font.pixelSize: 10
            Layout.fillWidth: true
            elide: Text.ElideRight
        }
    }
}
