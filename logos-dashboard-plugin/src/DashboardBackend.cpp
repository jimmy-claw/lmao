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
    QString json = callFfiStr(lmao_get_info());
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
    m_publicKey = obj.value("public_key").toString();
    m_taskTopic = obj.value("task_topic").toString();
    m_discoveryTopic = obj.value("discovery_topic").toString();
    m_presenceTopic = obj.value("presence_topic").toString();
    m_encryption = obj.value("encryption").toBool();
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
    obj.remove("success");
    m_peersDiscovered = obj.value("peers_discovered").toInt();
    emit metricsChanged(obj);
}

QJsonObject DashboardBackend::getMetrics()
{
    QString json = callFfiStr(lmao_get_metrics());
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    if (!doc.isObject()) return {};
    QJsonObject obj = doc.object();
    obj.remove("success");
    return obj;
}

QString DashboardBackend::getAgentCardJson()
{
    return callFfiStr(lmao_get_agent_card());
}
