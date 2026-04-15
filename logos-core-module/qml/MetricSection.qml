import QtQuick 2.15
import QtQuick.Layouts 1.15

Rectangle {
    id: section
    property string title: ""
    default property alias content: grid.children

    height: sectionCol.implicitHeight + Theme.spacing.large * 2
    radius: Theme.spacing.radiusLarge
    color: Theme.palette.backgroundTertiary
    border { color: Theme.palette.borderSecondary; width: 1 }

    ColumnLayout {
        id: sectionCol
        anchors {
            left: parent.left; right: parent.right; top: parent.top
            margins: Theme.spacing.large
        }
        spacing: Theme.spacing.small

        Text {
            text: section.title
            color: Theme.palette.textSecondary
            font { pixelSize: 12; bold: true }
        }

        GridLayout {
            id: grid
            Layout.fillWidth: true
            columns: 4
            columnSpacing: Theme.spacing.small
            rowSpacing: Theme.spacing.small
        }
    }
}
