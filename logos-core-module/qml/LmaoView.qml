import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * LmaoView — main tabbed view for LMAO agent dashboard.
 *
 * Context properties (injected by LmaoComponent):
 *   - lmaoAgentModel : AgentListModel*
 *   - lmaoModule     : LmaoBackend*
 *   - lmaoDelivery   : DeliveryTransport*
 */
Item {
    id: root

    property int currentTab: 0
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
        height: 44
        color: Theme.palette.backgroundSecondary

        RowLayout {
            anchors { fill: parent; leftMargin: Theme.spacing.medium; rightMargin: Theme.spacing.medium }
            spacing: 0

            Text {
                text: "LMAO"
                color: Theme.palette.primary
                font { pixelSize: 15; bold: true; family: "monospace" }
                Layout.rightMargin: Theme.spacing.medium
            }

            Repeater {
                model: ["Dashboard", "Fleet", "Metrics", "History"]
                delegate: Rectangle {
                    Layout.preferredHeight: 44
                    Layout.preferredWidth: tabLabel.implicitWidth + Theme.spacing.large * 2
                    color: "transparent"

                    Rectangle {
                        anchors { bottom: parent.bottom; left: parent.left; right: parent.right }
                        height: 2
                        color: Theme.palette.primary
                        visible: root.currentTab === index
                    }

                    Text {
                        id: tabLabel
                        anchors.centerIn: parent
                        text: modelData
                        color: root.currentTab === index ? Theme.palette.primary : Theme.palette.textSecondary
                        font { pixelSize: 12; bold: root.currentTab === index }
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.currentTab = index
                    }
                }
            }

            Item { Layout.fillWidth: true }

            // Online indicator
            Rectangle {
                width: 6; height: 6; radius: 3
                color: Theme.palette.success
            }
        }
    }

    // ── Tab Content ────────────────────────────────────────────────────
    StackLayout {
        anchors {
            left: parent.left; right: parent.right
            top: header.bottom; bottom: parent.bottom
        }
        currentIndex: root.currentTab

        // Tab 0: Dashboard
        DashboardView {}

        // Tab 1: Fleet (discovery + task sending)
        Item {
            ColumnLayout {
                anchors {
                    fill: parent
                    margins: Theme.spacing.medium
                }
                spacing: Theme.spacing.medium

                // ── Fleet header ──
                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.spacing.small

                    Text {
                        text: "Fleet Discovery"
                        color: Theme.palette.text
                        font { pixelSize: 16; bold: true }
                    }

                    Text {
                        visible: lmaoAgentModel.count > 0
                        text: lmaoAgentModel.count + " peers"
                        color: Theme.palette.textSecondary
                        font.pixelSize: 12
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

                // ── Agent List ──
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

                // ── Send Task Form ──
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
                            text: "Send Task"
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
                                    var result = lmaoModule.sendTaskViaDelivery(taskPubkeyField.text, taskTextField.text)
                                    root.lastResult = result
                                    // Record in task history
                                    if (historyView.item)
                                        historyView.item.addTask(taskPubkeyField.text, taskTextField.text, "sent", result)
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
            }
        }

        // Tab 2: Metrics
        MetricsView {}

        // Tab 3: Task History
        TaskHistoryView {
            id: historyView
        }
    }
}
