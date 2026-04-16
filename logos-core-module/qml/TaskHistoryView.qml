import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * TaskHistoryView — list of recent tasks sent/received.
 *
 * Context properties (injected by LmaoComponent):
 *   - lmaoModule     : LmaoBackend*
 *   - lmaoAgentModel : AgentListModel*
 */
Item {
    id: root

    // In-memory task history (most recent first)
    property var taskHistory: []
    property int selectedIndex: -1

    function addTask(peer, text, direction, resultJson) {
        var entry = {
            timestamp: new Date().toISOString(),
            peer: peer,
            text: text,
            direction: direction,  // "sent" or "received"
            status: "unknown",
            taskId: "",
            duration: 0
        }

        try {
            var obj = JSON.parse(resultJson)
            entry.status = obj.success ? "ok" : "failed"
            entry.taskId = obj.task_id || ""
            if (obj.error) entry.text = obj.error
        } catch (e) {
            entry.status = "error"
        }

        var updated = [entry].concat(taskHistory)
        // Keep max 50 entries
        if (updated.length > 50) updated = updated.slice(0, 50)
        taskHistory = updated
    }

    ColumnLayout {
        anchors {
            fill: parent
            margins: Theme.spacing.large
        }
        spacing: Theme.spacing.medium

        // ── Header ──
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing.small

            Text {
                text: "Task History"
                color: Theme.palette.text
                font { pixelSize: 16; bold: true }
            }

            Item { Layout.fillWidth: true }

            Text {
                text: taskHistory.length + " tasks"
                color: Theme.palette.textTertiary
                font.pixelSize: 12
            }

            Button {
                text: "Clear"
                visible: taskHistory.length > 0
                onClicked: {
                    taskHistory = []
                    selectedIndex = -1
                }
                contentItem: Text {
                    text: parent.text
                    color: Theme.palette.error
                    font.pixelSize: 11
                    horizontalAlignment: Text.AlignHCenter
                }
                background: Rectangle {
                    color: parent.hovered ? Theme.palette.backgroundSecondary : "transparent"
                    radius: Theme.spacing.tiny
                    border { color: Theme.palette.error; width: 1 }
                }
                height: 26
            }
        }

        // ── Task List ──
        ListView {
            id: historyList
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.spacing.tiny
            clip: true
            model: taskHistory.length

            ScrollBar.vertical: ScrollBar {}

            delegate: Rectangle {
                width: historyList.width
                height: taskCol.implicitHeight + Theme.spacing.medium * 2
                radius: Theme.spacing.radiusLarge
                color: index === root.selectedIndex
                       ? Theme.palette.backgroundSecondary
                       : Theme.palette.backgroundTertiary
                border {
                    color: index === root.selectedIndex
                           ? Theme.palette.primary
                           : Theme.palette.borderSecondary
                    width: 1
                }

                property var task: taskHistory[index] || ({})

                MouseArea {
                    anchors.fill: parent
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        root.selectedIndex = (root.selectedIndex === index) ? -1 : index
                    }
                }

                ColumnLayout {
                    id: taskCol
                    anchors {
                        left: parent.left; right: parent.right; top: parent.top
                        margins: Theme.spacing.medium
                    }
                    spacing: Theme.spacing.tiny

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.spacing.small

                        // Direction indicator
                        Rectangle {
                            width: 6; height: 6; radius: 3
                            color: task.direction === "sent" ? Theme.palette.primary : Theme.palette.success
                        }

                        Text {
                            text: task.direction === "sent" ? "→" : "←"
                            color: Theme.palette.textSecondary
                            font { pixelSize: 12; family: "monospace" }
                        }

                        Text {
                            text: {
                                var p = task.peer || ""
                                if (p.length > 16)
                                    return p.substring(0, 8) + "…" + p.slice(-6)
                                return p || "(unknown)"
                            }
                            color: Theme.palette.text
                            font { pixelSize: 13; family: "monospace" }
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        // Status badge
                        Rectangle {
                            width: statusText.implicitWidth + 12
                            height: 18
                            radius: 9
                            color: task.status === "ok" ? "#1B5E20"
                                 : task.status === "failed" ? "#B71C1C"
                                 : Theme.palette.backgroundSecondary

                            Text {
                                id: statusText
                                anchors.centerIn: parent
                                text: task.status || "?"
                                color: "#FFFFFF"
                                font.pixelSize: 10
                            }
                        }

                        Text {
                            text: {
                                if (!task.timestamp) return ""
                                var d = new Date(task.timestamp)
                                return Qt.formatTime(d, "HH:mm:ss")
                            }
                            color: Theme.palette.textTertiary
                            font.pixelSize: 11
                        }
                    }

                    // ── Expanded details ──
                    ColumnLayout {
                        visible: index === root.selectedIndex
                        Layout.fillWidth: true
                        spacing: 2

                        Text {
                            visible: (task.text || "").length > 0
                            text: "Text: " + task.text
                            color: Theme.palette.textSecondary
                            font.pixelSize: 11
                            wrapMode: Text.WordWrap
                            Layout.fillWidth: true
                        }

                        Text {
                            visible: (task.taskId || "").length > 0
                            text: "Task ID: " + task.taskId
                            color: Theme.palette.textTertiary
                            font { pixelSize: 11; family: "monospace" }
                            Layout.fillWidth: true
                            elide: Text.ElideRight
                        }

                        Text {
                            text: "Timestamp: " + (task.timestamp || "—")
                            color: Theme.palette.textTertiary
                            font.pixelSize: 11
                        }
                    }
                }
            }

            // ── Empty state ──
            Column {
                anchors.centerIn: parent
                spacing: Theme.spacing.medium
                visible: taskHistory.length === 0

                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "No tasks yet"
                    color: Theme.palette.textTertiary
                    font.pixelSize: 15
                }
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    text: "Tasks will appear here as you send and receive them"
                    color: Theme.palette.textTertiary
                    font.pixelSize: 12
                }
            }
        }
    }
}
