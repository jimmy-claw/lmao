#pragma once

#include <QAbstractListModel>
#include <QString>
#include <QList>

/**
 * AgentListModel — QAbstractListModel for discovered A2A agents.
 */
class AgentListModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Roles {
        NameRole = Qt::UserRole + 1,
        DescriptionRole,
        VersionRole,
        PubkeyRole,
        CapabilitiesRole,
    };
    Q_ENUM(Roles)

    explicit AgentListModel(QObject* parent = nullptr);
    ~AgentListModel() override = default;

    int rowCount(const QModelIndex& parent = QModelIndex()) const override;
    QVariant data(const QModelIndex& index, int role = Qt::DisplayRole) const override;
    QHash<int, QByteArray> roleNames() const override;

    Q_INVOKABLE void loadFromJson(const QString& json);
    Q_INVOKABLE void clear();

signals:
    void countChanged();

private:
    struct Agent {
        QString name;
        QString description;
        QString version;
        QString pubkey;
        QStringList capabilities;
    };

    QList<Agent> m_agents;
};
