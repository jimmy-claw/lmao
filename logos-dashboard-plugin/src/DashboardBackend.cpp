#include "DashboardBackend.h"

#include <QJsonDocument>
#include <QJsonObject>
#include <QDebug>

DashboardBackend::DashboardBackend(QObject *parent)
    : QObject(parent)
{
    char *v = lmao_version();
    m_version = callFfiStr(v);
}

QString DashboardBackend::callFfiStr(char *raw)
{
    if (!raw) return {};
    QString s = QString::fromUtf8(raw);
    lmao_free_string(raw);
    return s;
}

void DashboardBackend::refreshInfo()
{
    QString json = callFfiStr(lmao_get_node_info());
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    if (!doc.isObject()) {
        emit errorOccurred(QStringLiteral("Failed to parse info response"));
        return;
    }
    QJsonObject obj = doc.object();
    if (!obj.value("success").toBool()) {
        emit errorOccurred(obj.value("error").toString());
        return;
    }
    QJsonObject info = obj.value("info").toObject();
    m_publicKey = info.value("public_key").toString();
    m_encryption = info.value("encrypted").toBool();

    QJsonObject topics = info.value("topics").toObject();
    m_taskTopic = topics.value("task").toString();
    m_discoveryTopic = topics.value("discovery").toString();
    m_presenceTopic = topics.value("presence").toString();
    emit infoChanged();
}

void DashboardBackend::refreshMetrics()
{
    QString json = callFfiStr(lmao_get_metrics());
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    if (!doc.isObject()) {
        emit errorOccurred(QStringLiteral("Failed to parse metrics response"));
        return;
    }
    QJsonObject obj = doc.object();
    if (!obj.value("success").toBool()) {
        emit errorOccurred(obj.value("error").toString());
        return;
    }
    QJsonObject metrics = obj.value("metrics").toObject();
    m_peersDiscovered = metrics.value("peers_discovered").toInt();
    emit metricsChanged(metrics);
}

QJsonObject DashboardBackend::getMetrics()
{
    QString json = callFfiStr(lmao_get_metrics());
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    if (!doc.isObject()) return {};
    QJsonObject obj = doc.object();
    return obj.value("metrics").toObject();
}

QString DashboardBackend::getAgentCardJson()
{
    return callFfiStr(lmao_get_agent_card());
}
