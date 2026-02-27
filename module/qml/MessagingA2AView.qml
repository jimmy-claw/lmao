import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15

Rectangle {
    id: root
    color: "#1a1a2e"

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 12

        // Header
        Text {
            text: "🦞 Logos A2A Messaging"
            font.pixelSize: 24
            font.bold: true
            color: "#e94560"
            Layout.alignment: Qt.AlignHCenter
        }

        // Status bar
        Rectangle {
            Layout.fillWidth: true
            height: 40
            radius: 8
            color: a2aBackend.ready ? "#0f3460" : "#533a3a"

            RowLayout {
                anchors.fill: parent
                anchors.margins: 8
                Text {
                    text: a2aBackend.ready ? "● Connected" : "○ Disconnected"
                    color: a2aBackend.ready ? "#4ecca3" : "#e94560"
                    font.pixelSize: 14
                }
                Text {
                    text: a2aBackend.ready ? "Key: " + a2aBackend.pubkey.substring(0, 16) + "..." : ""
                    color: "#888"
                    font.pixelSize: 12
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignRight
                }
            }
        }

        // Init controls
        GroupBox {
            Layout.fillWidth: true
            title: "Initialize"
            label: Text { text: parent.title; color: "#ccc"; font.pixelSize: 14 }
            background: Rectangle { color: "#16213e"; radius: 8; border.color: "#0f3460" }

            ColumnLayout {
                anchors.fill: parent
                spacing: 8

                TextField {
                    id: agentName
                    placeholderText: "Agent name"
                    text: "my-logos-agent"
                    Layout.fillWidth: true
                    color: "#eee"
                    background: Rectangle { color: "#1a1a2e"; radius: 4; border.color: "#0f3460" }
                }
                TextField {
                    id: nwakuUrl
                    placeholderText: "nwaku REST URL"
                    text: "http://127.0.0.1:8645"
                    Layout.fillWidth: true
                    color: "#eee"
                    background: Rectangle { color: "#1a1a2e"; radius: 4; border.color: "#0f3460" }
                }
                RowLayout {
                    Button {
                        text: "Initialize"
                        enabled: !a2aBackend.ready
                        onClicked: a2aBackend.initialize(agentName.text, "A2A agent", nwakuUrl.text, true)
                    }
                    Button {
                        text: "Announce"
                        enabled: a2aBackend.ready
                        onClicked: a2aBackend.announce()
                    }
                    Button {
                        text: "Discover"
                        enabled: a2aBackend.ready
                        onClicked: a2aBackend.discover()
                    }
                }
            }
        }

        // Discovered agents
        GroupBox {
            Layout.fillWidth: true
            Layout.fillHeight: true
            title: "Discovered Agents (" + a2aBackend.agents.length + ")"
            label: Text { text: parent.title; color: "#ccc"; font.pixelSize: 14 }
            background: Rectangle { color: "#16213e"; radius: 8; border.color: "#0f3460" }

            ListView {
                anchors.fill: parent
                model: a2aBackend.agents
                clip: true
                delegate: Rectangle {
                    width: parent ? parent.width : 0
                    height: 40
                    color: index % 2 === 0 ? "#1a1a2e" : "#16213e"
                    radius: 4

                    Text {
                        anchors.fill: parent
                        anchors.margins: 8
                        text: JSON.stringify(modelData)
                        color: "#ccc"
                        font.pixelSize: 12
                        elide: Text.ElideRight
                        verticalAlignment: Text.AlignVCenter
                    }
                }
            }
        }

        // Send message
        GroupBox {
            Layout.fillWidth: true
            title: "Send Message"
            label: Text { text: parent.title; color: "#ccc"; font.pixelSize: 14 }
            background: Rectangle { color: "#16213e"; radius: 8; border.color: "#0f3460" }

            RowLayout {
                anchors.fill: parent
                TextField {
                    id: targetPubkey
                    placeholderText: "Target pubkey"
                    Layout.fillWidth: true
                    color: "#eee"
                    background: Rectangle { color: "#1a1a2e"; radius: 4; border.color: "#0f3460" }
                }
                TextField {
                    id: messageText
                    placeholderText: "Message"
                    Layout.fillWidth: true
                    color: "#eee"
                    background: Rectangle { color: "#1a1a2e"; radius: 4; border.color: "#0f3460" }
                }
                Button {
                    text: "Send"
                    enabled: a2aBackend.ready
                    onClicked: {
                        a2aBackend.sendText(targetPubkey.text, messageText.text)
                        messageText.text = ""
                    }
                }
            }
        }
    }

    // Error popup
    Connections {
        target: a2aBackend
        function onErrorOccurred(error) {
            errorLabel.text = error
            errorTimer.restart()
        }
    }
    Text {
        id: errorLabel
        anchors.bottom: parent.bottom
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.margins: 8
        color: "#e94560"
        font.pixelSize: 12
        Timer { id: errorTimer; interval: 5000; onTriggered: errorLabel.text = "" }
    }
}
