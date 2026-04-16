#include "LmaoBackend.h"
#include "DeliveryTransport.h"

#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>
#include <QByteArray>

#ifdef __cplusplus
extern "C" {
#endif
#include "lmao_ffi.h"
#ifdef __cplusplus
}
#endif

LmaoBackend::LmaoBackend(DeliveryTransport* delivery, QObject* parent)
    : QObject(parent)
    , m_delivery(delivery)
{
    if (m_delivery) {
        qDebug() << "LmaoBackend: using QtRO delivery transport";
    } else {
        qDebug() << "LmaoBackend: no QtRO transport — using FFI only";
    }
}

/*static*/
QString LmaoBackend::callFfiStr(char* raw)
{
    if (!raw)
        return QStringLiteral(R"({"success":false,"error":"FFI returned null"})");
    QString result = QString::fromUtf8(raw);
    lmao_free_string(raw);
    return result;
}

QString LmaoBackend::discoverAgents(const QString& timeoutMs)
{
    qDebug() << "LmaoBackend::discoverAgents timeout=" << timeoutMs;

    QJsonObject obj;
    bool ok = false;
    int timeout = timeoutMs.toInt(&ok);
    obj[QLatin1String("timeout_ms")] = ok ? timeout : 5000;
    const QByteArray argsUtf8 = QJsonDocument(obj).toJson(QJsonDocument::Compact);

    const QString result = callFfiStr(lmao_discover_agents(argsUtf8.constData()));
    emit agentsDiscovered(result);
    return result;
}

QString LmaoBackend::sendTask(const QString& agentPubkey, const QString& taskText)
{
    qDebug() << "LmaoBackend::sendTask to=" << agentPubkey;

    QJsonObject obj;
    obj[QLatin1String("agent_pubkey")] = agentPubkey;
    obj[QLatin1String("task_text")] = taskText;
    const QByteArray argsUtf8 = QJsonDocument(obj).toJson(QJsonDocument::Compact);

    const QString result = callFfiStr(lmao_send_task(argsUtf8.constData()));
    emit taskSent(result);
    return result;
}

QString LmaoBackend::getAgentCard()
{
    qDebug() << "LmaoBackend::getAgentCard";
    return callFfiStr(lmao_get_agent_card());
}

QString LmaoBackend::getInfo()
{
    qDebug() << "LmaoBackend::getInfo";
    return callFfiStr(lmao_get_info());
}

QString LmaoBackend::getMetrics()
{
    qDebug() << "LmaoBackend::getMetrics";
    return callFfiStr(lmao_get_metrics());
}

bool LmaoBackend::deliverySend(const QString& contentTopic, const QByteArray& payload)
{
    if (!m_delivery) {
        qDebug() << "LmaoBackend::deliverySend — no QtRO transport available";
        return false;
    }

    bool ok = m_delivery->send(contentTopic, payload);
    if (!ok) {
        emit errorOccurred(QStringLiteral("delivery_module send failed via QtRO"));
    }
    return ok;
}
