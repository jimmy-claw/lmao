import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

/**
 * MetricsView — Real-time operational counters for the LMAO node.
 *
 * Context properties (injected by LmaoComponent):
 *   - lmaoModule : LmaoBackend*
 */
Item {
    id: metricsRoot

    property var metrics: ({})
    property bool autoRefresh: false

    function refresh() {
        var raw = lmaoModule.getMetrics()
        try {
            var obj = JSON.parse(raw)
            if (obj.success) metrics = obj.metrics
        } catch(e) {}
    }

    Component.onCompleted: refresh()

    Timer {
        id: pollTimer
        interval: 2000
        repeat: true
        running: metricsRoot.autoRefresh
        onTriggered: metricsRoot.refresh()
    }

    Flickable {
        anchors.fill: parent
        anchors.margins: Theme.spacing.medium
        contentHeight: col.implicitHeight
        clip: true

        ColumnLayout {
            id: col
            width: parent.width
            spacing: Theme.spacing.medium

            // ── Header row ──────────────────────────────────────────────
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacing.small

                Text {
                    text: "Metrics"
                    color: Theme.palette.primary
                    font { pixelSize: 18; bold: true; family: "monospace" }
                }

                Item { Layout.fillWidth: true }

                Switch {
                    id: autoSwitch
                    checked: metricsRoot.autoRefresh
                    onToggled: metricsRoot.autoRefresh = checked
                }
                Text {
                    text: "Auto"
                    color: Theme.palette.textSecondary
                    font.pixelSize: 11
                }

                Button {
                    text: "Refresh"
                    onClicked: metricsRoot.refresh()
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

            // ── Tasks ───────────────────────────────────────────────────
            MetricSection {
                title: "Tasks"
                Layout.fillWidth: true

                MetricCounter { label: "Sent";     value: metrics.tasks_sent || 0;     accent: Theme.palette.primary }
                MetricCounter { label: "Received"; value: metrics.tasks_received || 0;  accent: Theme.palette.success }
                MetricCounter { label: "Failed";   value: metrics.tasks_failed || 0;    accent: metrics.tasks_failed > 0 ? Theme.palette.error : Theme.palette.textTertiary }
                MetricCounter { label: "Responses"; value: metrics.responses_sent || 0; accent: Theme.palette.primary }
            }

            // ── Messaging ───────────────────────────────────────────────
            MetricSection {
                title: "Messaging"
                Layout.fillWidth: true

                MetricCounter { label: "Published";  value: metrics.messages_published || 0; accent: Theme.palette.primary }
                MetricCounter { label: "Received";   value: metrics.messages_received || 0;  accent: Theme.palette.success }
                MetricCounter { label: "Chunks Sent"; value: metrics.stream_chunks_sent || 0; accent: Theme.palette.primary }
                MetricCounter { label: "Chunks Recv"; value: metrics.stream_chunks_received || 0; accent: Theme.palette.success }
            }

            // ── Discovery ───────────────────────────────────────────────
            MetricSection {
                title: "Discovery"
                Layout.fillWidth: true

                MetricCounter { label: "Discoveries";     value: metrics.discoveries || 0;         accent: Theme.palette.primary }
                MetricCounter { label: "Announcements";   value: metrics.announcements_sent || 0;  accent: Theme.palette.primary }
                MetricCounter { label: "Peers Found";     value: metrics.peers_discovered || 0;    accent: Theme.palette.success }
                MetricCounter { label: "Sessions";        value: metrics.sessions_created || 0;    accent: Theme.palette.primary }
            }

            // ── Encryption & Reliability ────────────────────────────────
            MetricSection {
                title: "Encryption & Reliability"
                Layout.fillWidth: true

                MetricCounter { label: "Encryptions"; value: metrics.encryptions || 0;     accent: Theme.palette.primary }
                MetricCounter { label: "Decryptions"; value: metrics.decryptions || 0;     accent: Theme.palette.success }
                MetricCounter { label: "Retries";     value: metrics.retry_attempts || 0;  accent: metrics.retry_attempts > 0 ? "#FF9800" : Theme.palette.textTertiary }
                MetricCounter { label: "Exhausted";   value: metrics.retries_exhausted || 0; accent: metrics.retries_exhausted > 0 ? Theme.palette.error : Theme.palette.textTertiary }
            }

            // ── Delegations ─────────────────────────────────────────────
            MetricSection {
                title: "Delegations"
                Layout.fillWidth: true

                MetricCounter { label: "Sent"; value: metrics.delegations_sent || 0; accent: Theme.palette.primary }
            }
        }
    }
}
