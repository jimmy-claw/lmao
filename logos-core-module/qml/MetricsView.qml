import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * MetricsView — operational metrics counters from the LMAO node.
 *
 * Context properties (injected by LmaoComponent):
 *   - lmaoModule : LmaoBackend*
 */
Item {
    id: root

    property var metricsData: ({})
    property bool loaded: false
    property string errorMsg: ""

    function refresh() {
        errorMsg = ""
        var result = lmaoModule.getMetrics()
        try {
            var obj = JSON.parse(result)
            if (obj.success) {
                metricsData = obj
                loaded = true
            } else {
                errorMsg = obj.error || "Unknown error"
            }
        } catch (e) {
            errorMsg = "Failed to parse metrics response"
        }
    }

    // Auto-refresh every 5 seconds
    Timer {
        id: autoRefresh
        interval: 5000
        running: true
        repeat: true
        onTriggered: root.refresh()
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

            // ── Header ──
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacing.small

                Text {
                    text: "Metrics"
                    color: Theme.palette.text
                    font { pixelSize: 16; bold: true }
                }

                Item { Layout.fillWidth: true }

                Text {
                    visible: root.loaded
                    text: "Auto-refresh: 5s"
                    color: Theme.palette.textTertiary
                    font.pixelSize: 10
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

            // ── Error display ──
            Text {
                visible: root.errorMsg.length > 0
                text: "⚠ " + root.errorMsg
                color: Theme.palette.error
                font.pixelSize: 12
                Layout.fillWidth: true
            }

            // ── Tasks section ──
            MetricSection {
                title: "Tasks"
                metrics: [
                    { label: "Sent",     value: metricsData.tasks_sent || 0 },
                    { label: "Received", value: metricsData.tasks_received || 0 },
                    { label: "Failed",   value: metricsData.tasks_failed || 0,
                      highlight: (metricsData.tasks_failed || 0) > 0 }
                ]
            }

            // ── Messages section ──
            MetricSection {
                title: "Messages"
                metrics: [
                    { label: "Published", value: metricsData.messages_published || 0 },
                    { label: "Received",  value: metricsData.messages_received || 0 },
                    { label: "Responses",  value: metricsData.responses_sent || 0 }
                ]
            }

            // ── Discovery section ──
            MetricSection {
                title: "Discovery"
                metrics: [
                    { label: "Discoveries",   value: metricsData.discoveries || 0 },
                    { label: "Announcements",  value: metricsData.announcements_sent || 0 },
                    { label: "Peers Found",    value: metricsData.peers_discovered || 0 }
                ]
            }

            // ── Sessions & Encryption section ──
            MetricSection {
                title: "Sessions & Encryption"
                metrics: [
                    { label: "Sessions",    value: metricsData.sessions_created || 0 },
                    { label: "Encryptions", value: metricsData.encryptions || 0 },
                    { label: "Decryptions", value: metricsData.decryptions || 0 }
                ]
            }

            // ── Streaming section ──
            MetricSection {
                title: "Streaming & Delegation"
                metrics: [
                    { label: "Chunks Sent",     value: metricsData.stream_chunks_sent || 0 },
                    { label: "Chunks Received", value: metricsData.stream_chunks_received || 0 },
                    { label: "Delegations",     value: metricsData.delegations_sent || 0 }
                ]
            }

            // ── Retries section ──
            MetricSection {
                title: "Retries"
                metrics: [
                    { label: "Attempts",  value: metricsData.retry_attempts || 0 },
                    { label: "Exhausted", value: metricsData.retries_exhausted || 0,
                      highlight: (metricsData.retries_exhausted || 0) > 0 }
                ]
            }
        }
    }

    // ── MetricSection helper component ──
    component MetricSection: Rectangle {
        property string title: ""
        property var metrics: []

        Layout.fillWidth: true
        implicitHeight: sectionCol.implicitHeight + Theme.spacing.medium * 2
        radius: Theme.spacing.radiusLarge
        color: Theme.palette.backgroundTertiary
        border { color: Theme.palette.borderSecondary; width: 1 }

        ColumnLayout {
            id: sectionCol
            anchors {
                left: parent.left; right: parent.right; top: parent.top
                margins: Theme.spacing.medium
            }
            spacing: Theme.spacing.tiny

            Text {
                text: title
                color: Theme.palette.textSecondary
                font { pixelSize: 11; bold: true; letterSpacing: 1 }
            }

            // Counter tiles row
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacing.small

                Repeater {
                    model: metrics
                    delegate: Rectangle {
                        Layout.fillWidth: true
                        height: 56
                        radius: Theme.spacing.tiny
                        color: Theme.palette.backgroundSecondary

                        ColumnLayout {
                            anchors.centerIn: parent
                            spacing: 2

                            Text {
                                text: modelData.value !== undefined ? modelData.value.toString() : "0"
                                color: (modelData.highlight) ? Theme.palette.error : Theme.palette.primary
                                font { pixelSize: 20; bold: true; family: "monospace" }
                                Layout.alignment: Qt.AlignHCenter
                            }

                            Text {
                                text: modelData.label || ""
                                color: Theme.palette.textTertiary
                                font.pixelSize: 10
                                Layout.alignment: Qt.AlignHCenter
                            }
                        }
                    }
                }
            }
        }
    }
}
