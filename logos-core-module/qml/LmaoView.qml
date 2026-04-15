import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * LmaoView — main tabbed container for the LMAO dashboard plugin.
 *
 * Tabs: Dashboard | Fleet | Metrics
 *
 * Context properties (injected by LmaoComponent):
 *   - lmaoAgentModel : AgentListModel*
 *   - lmaoModule     : LmaoBackend*
 */
Item {
    id: root

    Rectangle {
        anchors.fill: parent
        color: Theme.palette.background
    }

    // ── Tab Bar ────────────────────────────────────────────────────────
    Rectangle {
        id: tabBar
        anchors { left: parent.left; right: parent.right; top: parent.top }
        height: 44
        color: Theme.palette.backgroundSecondary

        RowLayout {
            anchors {
                fill: parent
                leftMargin: Theme.spacing.large
                rightMargin: Theme.spacing.large
            }
            spacing: 0

            Text {
                text: "LMAO"
                color: Theme.palette.primary
                font { pixelSize: 16; bold: true; family: "monospace" }
                Layout.rightMargin: Theme.spacing.large
            }

            Repeater {
                model: ["Dashboard", "Fleet", "Metrics"]

                delegate: Rectangle {
                    property bool active: tabStack.currentIndex === index

                    Layout.preferredWidth: tabLabel.implicitWidth + Theme.spacing.large * 2
                    Layout.fillHeight: true
                    color: active ? Theme.palette.background : "transparent"
                    radius: Theme.spacing.tiny

                    Text {
                        id: tabLabel
                        anchors.centerIn: parent
                        text: modelData
                        color: active ? Theme.palette.primary : Theme.palette.textSecondary
                        font {
                            pixelSize: 13
                            bold: active
                        }
                    }

                    Rectangle {
                        visible: active
                        anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
                        height: 2
                        color: Theme.palette.primary
                    }

                    MouseArea {
                        anchors.fill: parent
                        cursorShape: Qt.PointingHandCursor
                        onClicked: tabStack.currentIndex = index
                    }
                }
            }

            Item { Layout.fillWidth: true }

            // Online indicator
            Rectangle {
                width: 8; height: 8; radius: 4
                color: Theme.palette.success
                Layout.alignment: Qt.AlignVCenter
            }
            Text {
                text: "Online"
                color: Theme.palette.success
                font.pixelSize: 11
                Layout.alignment: Qt.AlignVCenter
            }
        }
    }

    // ── Content Stack ──────────────────────────────────────────────────
    StackLayout {
        id: tabStack
        anchors {
            left: parent.left; right: parent.right
            top: tabBar.bottom; bottom: parent.bottom
        }

        DashboardView { }
        FleetView { }
        MetricsView { }
    }
}
