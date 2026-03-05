#include "AgentListModel.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>

AgentListModel::AgentListModel(QObject* parent)
    : QAbstractListModel(parent)
{
}

int AgentListModel::rowCount(const QModelIndex& parent) const
{
    if (parent.isValid()) return 0;
    return m_agents.size();
}

QVariant AgentListModel::data(const QModelIndex& index, int role) const
{
    if (!index.isValid() || index.row() < 0 || index.row() >= m_agents.size())
        return {};

    const Agent& a = m_agents.at(index.row());
    switch (static_cast<Roles>(role)) {
        case NameRole:         return a.name;
        case DescriptionRole:  return a.description;
        case VersionRole:      return a.version;
        case PubkeyRole:       return a.pubkey;
        case CapabilitiesRole: return a.capabilities;
        default:               return {};
    }
}

QHash<int, QByteArray> AgentListModel::roleNames() const
{
    return {
        { NameRole,         "name"         },
        { DescriptionRole,  "description"  },
        { VersionRole,      "version"      },
        { PubkeyRole,       "pubkey"       },
        { CapabilitiesRole, "capabilities" },
    };
}

void AgentListModel::loadFromJson(const QString& json)
{
    QJsonParseError err;
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8(), &err);
    if (err.error != QJsonParseError::NoError) return;

    const QJsonObject root = doc.object();
    if (!root.value(QLatin1String("success")).toBool()) return;

    const QJsonArray agents = root.value(QLatin1String("agents")).toArray();
    QList<Agent> parsed;
    parsed.reserve(agents.size());

    for (const QJsonValue& v : agents) {
        if (!v.isObject()) continue;
        const QJsonObject a = v.toObject();
        Agent agent;
        agent.name        = a.value(QLatin1String("name")).toString();
        agent.description = a.value(QLatin1String("description")).toString();
        agent.version     = a.value(QLatin1String("version")).toString();
        agent.pubkey      = a.value(QLatin1String("public_key")).toString();
        const QJsonArray caps = a.value(QLatin1String("capabilities")).toArray();
        for (const QJsonValue& c : caps) {
            agent.capabilities << c.toString();
        }
        parsed.append(agent);
    }

    beginResetModel();
    m_agents = parsed;
    endResetModel();
    emit countChanged();
}

void AgentListModel::clear()
{
    beginResetModel();
    m_agents.clear();
    endResetModel();
    emit countChanged();
}
