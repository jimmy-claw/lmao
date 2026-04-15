import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

Item {
    id: root
    width: 480
    height: 720

    // ── State ──
    property var metrics: ({})
    property string lastError: ""

    // Poll timer for live metrics
    Timer {
        id: pollTimer
        interval: 3000
        repeat: true
        running: true
        triggeredOnStart: true
        onTriggered: {
            dashboardBackend.refreshInfo()
            dashboardBackend.refreshMetrics()
        }
    }

    Connections {
        target: dashboardBackend
        function onMetricsChanged(m) { root.metrics = m }
        function onErrorOccurred(err) {
            root.lastError = err
            errorTimer.restart()
        }
    }

    Timer {
        id: errorTimer
        interval: 5000
        onTriggered: root.lastError = ""
    }

    // ── Background ──
    Rectangle {
        anchors.fill: parent
        color: Theme.background
    }

    Flickable {
        anchors.fill: parent
        anchors.margins: Theme.large
        contentHeight: mainColumn.height
        clip: true

        ColumnLayout {
            id: mainColumn
            width: parent.width
            spacing: Theme.large

            // ════════════════════════════════════════════════
            // HEADER
            // ════════════════════════════════════════════════
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.medium

                Text {
                    text: "LMAO Dashboard"
                    font.pixelSize: 20
                    font.bold: true
                    color: Theme.textPrimary
                }

                Item { Layout.fillWidth: true }

                // Status indicator
                Rectangle {
                    width: 10; height: 10; radius: 5
                    color: dashboardBackend.publicKey !== "" ? Theme.success : Theme.error
                }
                Text {
                    text: dashboardBackend.publicKey !== "" ? "Online" : "Offline"
                    font.pixelSize: 12
                    color: Theme.textSecondary
                }

                // Version badge
                Rectangle {
                    width: verLabel.width + Theme.medium
                    height: 22
                    radius: 11
                    color: Theme.surface
                    border.color: Theme.border
                    Text {
                        id: verLabel
                        anchors.centerIn: parent
                        text: "v" + dashboardBackend.version
                        font.pixelSize: 10
                        color: Theme.textSecondary
                    }
                }
            }

            // ════════════════════════════════════════════════
            // AGENT IDENTITY
            // ════════════════════════════════════════════════
            Rectangle {
                Layout.fillWidth: true
                height: identityCol.height + Theme.xlarge
                radius: Theme.radiusLarge
                color: Theme.surface
                border.color: Theme.border

                Column {
                    id: identityCol
                    anchors {
                        left: parent.left; right: parent.right
                        top: parent.top; margins: Theme.medium
                    }
                    spacing: Theme.small

                    Text {
                        text: "Agent Identity"
                        font.pixelSize: 14
                        font.bold: true
                        color: Theme.primary
                    }

                    // Public key row
                    Row {
                        spacing: Theme.small
                        Text {
                            text: "Public Key:"
                            font.pixelSize: 12
                            color: Theme.textSecondary
                            width: 100
                        }
                        Text {
                            text: {
                                var pk = dashboardBackend.publicKey
                                if (pk.length > 16)
                                    return pk.substring(0, 8) + "..." + pk.substring(pk.length - 8)
                                return pk || "---"
                            }
                            font.pixelSize: 12
                            font.family: "monospace"
                            color: Theme.textPrimary

                            MouseArea {
                                anchors.fill: parent
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    // Copy full pubkey — requires clipboard helper in production
                                }
                            }
                        }
                    }

                    // Encryption
                    Row {
                        spacing: Theme.small
                        Text {
                            text: "Encryption:"
                            font.pixelSize: 12
                            color: Theme.textSecondary
                            width: 100
                        }
                        Text {
                            text: dashboardBackend.encryption ? "Enabled" : "Disabled"
                            font.pixelSize: 12
                            color: dashboardBackend.encryption ? Theme.success : Theme.warning
                        }
                    }
                }
            }

            // ════════════════════════════════════════════════
            // WAKU TOPICS
            // ════════════════════════════════════════════════
            Rectangle {
                Layout.fillWidth: true
                height: topicsCol.height + Theme.xlarge
                radius: Theme.radiusLarge
                color: Theme.surface
                border.color: Theme.border

                Column {
                    id: topicsCol
                    anchors {
                        left: parent.left; right: parent.right
                        top: parent.top; margins: Theme.medium
                    }
                    spacing: Theme.small

                    Text {
                        text: "Waku Topics"
                        font.pixelSize: 14
                        font.bold: true
                        color: Theme.primary
                    }

                    Repeater {
                        model: [
                            { label: "Task", value: dashboardBackend.taskTopic },
                            { label: "Discovery", value: dashboardBackend.discoveryTopic },
                            { label: "Presence", value: dashboardBackend.presenceTopic }
                        ]
                        delegate: Row {
                            spacing: Theme.small
                            Text {
                                text: modelData.label + ":"
                                font.pixelSize: 11
                                color: Theme.textSecondary
                                width: 80
                            }
                            Text {
                                text: modelData.value || "---"
                                font.pixelSize: 11
                                font.family: "monospace"
                                color: Theme.textTertiary
                                elide: Text.ElideMiddle
                                width: 300
                            }
                        }
                    }
                }
            }

            // ════════════════════════════════════════════════
            // METRICS — TASKS
            // ════════════════════════════════════════════════
            Text {
                text: "Task Metrics"
                font.pixelSize: 14
                font.bold: true
                color: Theme.primary
            }

            Flow {
                Layout.fillWidth: true
                spacing: Theme.small

                MetricCard {
                    label: "Sent"
                    value: root.metrics.tasks_sent || 0
                    valueColor: Theme.accent
                }
                MetricCard {
                    label: "Received"
                    value: root.metrics.tasks_received || 0
                    valueColor: Theme.success
                }
                MetricCard {
                    label: "Failed"
                    value: root.metrics.tasks_failed || 0
                    valueColor: Theme.error
                }
                MetricCard {
                    label: "Responses"
                    value: root.metrics.responses_sent || 0
                    valueColor: Theme.textPrimary
                }
            }

            // ════════════════════════════════════════════════
            // METRICS — MESSAGING
            // ════════════════════════════════════════════════
            Text {
                text: "Messaging"
                font.pixelSize: 14
                font.bold: true
                color: Theme.primary
            }

            Flow {
                Layout.fillWidth: true
                spacing: Theme.small

                MetricCard {
                    label: "Published"
                    value: root.metrics.messages_published || 0
                }
                MetricCard {
                    label: "Received"
                    value: root.metrics.messages_received || 0
                }
                MetricCard {
                    label: "Discoveries"
                    value: root.metrics.discoveries || 0
                }
                MetricCard {
                    label: "Peers"
                    value: root.metrics.peers_discovered || 0
                    valueColor: Theme.accent
                }
            }

            // ════════════════════════════════════════════════
            // METRICS — CRYPTO & SESSIONS
            // ════════════════════════════════════════════════
            Text {
                text: "Crypto & Sessions"
                font.pixelSize: 14
                font.bold: true
                color: Theme.primary
            }

            Flow {
                Layout.fillWidth: true
                spacing: Theme.small

                MetricCard {
                    label: "Encryptions"
                    value: root.metrics.encryptions || 0
                }
                MetricCard {
                    label: "Decryptions"
                    value: root.metrics.decryptions || 0
                }
                MetricCard {
                    label: "Sessions"
                    value: root.metrics.sessions_created || 0
                }
                MetricCard {
                    label: "Retries"
                    value: root.metrics.retry_attempts || 0
                    valueColor: root.metrics.retries_exhausted > 0 ? Theme.warning : Theme.textPrimary
                }
            }

            // ════════════════════════════════════════════════
            // METRICS — STREAMING & DELEGATION
            // ════════════════════════════════════════════════
            Text {
                text: "Streaming & Delegation"
                font.pixelSize: 14
                font.bold: true
                color: Theme.primary
            }

            Flow {
                Layout.fillWidth: true
                spacing: Theme.small

                MetricCard {
                    label: "Delegations"
                    value: root.metrics.delegations_sent || 0
                }
                MetricCard {
                    label: "Chunks Sent"
                    value: root.metrics.stream_chunks_sent || 0
                }
                MetricCard {
                    label: "Chunks Recv"
                    value: root.metrics.stream_chunks_received || 0
                }
                MetricCard {
                    label: "Announcements"
                    value: root.metrics.announcements_sent || 0
                }
            }

            // ════════════════════════════════════════════════
            // ERROR BANNER
            // ════════════════════════════════════════════════
            Rectangle {
                Layout.fillWidth: true
                height: 36
                radius: Theme.radiusSmall
                color: "#4DF44336"
                visible: root.lastError !== ""

                Text {
                    anchors.centerIn: parent
                    text: root.lastError
                    font.pixelSize: 12
                    color: Theme.error
                    elide: Text.ElideRight
                    width: parent.width - Theme.large
                }
            }

            // Bottom spacer
            Item { height: Theme.large }
        }
    }
}
