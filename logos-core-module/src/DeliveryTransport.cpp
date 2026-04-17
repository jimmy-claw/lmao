#include "DeliveryTransport.h"

#include <QDebug>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonArray>
#include <QMetaObject>
#include <QVariant>

DeliveryTransport::DeliveryTransport(QObject* parent)
    : QObject(parent)
{
}

bool DeliveryTransport::init(LogosAPI* logosAPI)
{
    if (!logosAPI) {
        qWarning() << "DeliveryTransport::init — logosAPI is null";
        return false;
    }

    // logosAPI is a QObject-derived class from the Logos SDK.
    // Use QMetaObject::invokeMethod to call getClient("delivery_module")
    // without needing the full LogosAPI header at compile time.
    auto* apiObj = reinterpret_cast<QObject*>(logosAPI);

    QObject* client = nullptr;
    bool ok = QMetaObject::invokeMethod(
        apiObj,
        "getClient",
        Qt::DirectConnection,
        Q_RETURN_ARG(QObject*, client),
        Q_ARG(QString, QStringLiteral("delivery_module")));

    if (!ok || !client) {
        qWarning() << "DeliveryTransport::init — failed to get delivery_module client"
                    << "(invokeMethod returned" << ok << ")";
        return false;
    }

    m_client = client;
    qDebug() << "DeliveryTransport::init — connected to delivery_module";

    // Connect the messageReceived event from the delivery_module replica.
    // The delivery_module emits: void eventReceived(const QString& name, const QString& data)
    connect(m_client, SIGNAL(eventReceived(QString, QString)),
            this, SLOT(onDeliveryEvent(QString, QString)));

    emit connectedChanged();
    return true;
}

void DeliveryTransport::onDeliveryEvent(const QString& eventName, const QString& eventData)
{
    if (eventName != QLatin1String("messageReceived"))
        return;

    QJsonDocument doc = QJsonDocument::fromJson(eventData.toUtf8());
    if (!doc.isObject())
        return;

    QJsonObject obj = doc.object();
    QString topic = obj.value(QLatin1String("contentTopic")).toString();
    QString payloadB64 = obj.value(QLatin1String("payload")).toString();
    QByteArray payload = QByteArray::fromBase64(payloadB64.toUtf8());

    emit messageReceived(topic, payload);
}

QString DeliveryTransport::callMethod(const QString& method, const QVariantList& args)
{
    if (!m_client) {
        qWarning() << "DeliveryTransport::callMethod — not connected";
        return {};
    }

    // Build the Logos Core params JSON array:
    // [{"name":"key","value":"val","type":"string"}, ...]
    QJsonArray params;
    for (int i = 0; i + 1 < args.size(); i += 2) {
        QJsonObject param;
        param[QLatin1String("name")] = args[i].toString();
        param[QLatin1String("value")] = args[i + 1].toString();
        param[QLatin1String("type")] = QStringLiteral("string");
        params.append(param);
    }

    QString paramsJson = QString::fromUtf8(
        QJsonDocument(params).toJson(QJsonDocument::Compact));

    QString result;
    bool ok = QMetaObject::invokeMethod(
        m_client,
        "callMethod",
        Qt::DirectConnection,
        Q_RETURN_ARG(QString, result),
        Q_ARG(QString, method),
        Q_ARG(QString, paramsJson));

    if (!ok) {
        qWarning() << "DeliveryTransport::callMethod — invokeMethod failed for" << method;
        return {};
    }

    return result;
}

bool DeliveryTransport::createNode(const QString& configJson)
{
    QString result = callMethod(QStringLiteral("createNode"),
                                {QStringLiteral("cfg"), configJson});
    if (result != QLatin1String("true")) {
        qWarning() << "DeliveryTransport::createNode failed:" << result;
        return false;
    }
    return true;
}

bool DeliveryTransport::start()
{
    // start takes no params — pass empty list
    QString result = callMethod(QStringLiteral("start"), {});
    if (result != QLatin1String("true")) {
        qWarning() << "DeliveryTransport::start failed:" << result;
        return false;
    }
    return true;
}

bool DeliveryTransport::send(const QString& contentTopic, const QByteArray& payload)
{
    QString payloadB64 = QString::fromLatin1(payload.toBase64());
    QString result = callMethod(QStringLiteral("send"),
                                {QStringLiteral("contentTopic"), contentTopic,
                                 QStringLiteral("payload"), payloadB64});

    if (result.startsWith(QLatin1String("error"), Qt::CaseInsensitive)) {
        qWarning() << "DeliveryTransport::send failed:" << result;
        return false;
    }
    return true;
}

bool DeliveryTransport::subscribe(const QString& contentTopic)
{
    QString result = callMethod(QStringLiteral("subscribe"),
                                {QStringLiteral("contentTopic"), contentTopic});
    if (result != QLatin1String("true")) {
        qWarning() << "DeliveryTransport::subscribe failed:" << result;
        return false;
    }
    return true;
}

bool DeliveryTransport::unsubscribe(const QString& contentTopic)
{
    QString result = callMethod(QStringLiteral("unsubscribe"),
                                {QStringLiteral("contentTopic"), contentTopic});
    if (result != QLatin1String("true")) {
        qWarning() << "DeliveryTransport::unsubscribe failed:" << result;
        return false;
    }
    return true;
}
