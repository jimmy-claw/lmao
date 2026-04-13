#include "LmaoComponent.h"
#include "LmaoBackend.h"
#include "AgentListModel.h"

#include <QQuickWidget>
#include <QQmlContext>
#include <QDebug>
#include <QRemoteObjectDynamicReplica>
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

// ---------------------------------------------------------------------------
// QtRO → Rust bridge callbacks
//
// These static functions forward delivery_module QtRO calls to the Rust
// transport layer. The user_data pointer is the QRemoteObjectDynamicReplica*.
// ---------------------------------------------------------------------------

static int qtro_publish(const char* topic, const char* payload_b64, void* user_data)
{
    auto* replica = static_cast<QRemoteObjectDynamicReplica*>(user_data);
    if (!replica || !replica->isInitialized()) {
        qWarning() << "LMAO: delivery_module replica not ready for publish";
        return -1;
    }

    QJsonObject params;
    params[QLatin1String("contentTopic")] = QString::fromUtf8(topic);
    params[QLatin1String("payload")] = QString::fromUtf8(payload_b64);
    QByteArray paramsJson = QJsonDocument(params).toJson(QJsonDocument::Compact);

    // Call the send method on the delivery_module replica
    QMetaObject::invokeMethod(replica, "send",
        Q_ARG(QString, QString::fromUtf8(paramsJson)));

    return 0;
}

static int qtro_subscribe(const char* topic, void* user_data)
{
    auto* replica = static_cast<QRemoteObjectDynamicReplica*>(user_data);
    if (!replica || !replica->isInitialized()) {
        qWarning() << "LMAO: delivery_module replica not ready for subscribe";
        return -1;
    }

    QMetaObject::invokeMethod(replica, "subscribe",
        Q_ARG(QString, QString::fromUtf8(topic)));

    return 0;
}

static int qtro_unsubscribe(const char* topic, void* user_data)
{
    auto* replica = static_cast<QRemoteObjectDynamicReplica*>(user_data);
    if (!replica || !replica->isInitialized()) {
        qWarning() << "LMAO: delivery_module replica not ready for unsubscribe";
        return -1;
    }

    QMetaObject::invokeMethod(replica, "unsubscribe",
        Q_ARG(QString, QString::fromUtf8(topic)));

    return 0;
}

// ---------------------------------------------------------------------------

LmaoComponent::LmaoComponent(QObject* parent)
    : QObject(parent)
{
    qDebug() << "LmaoComponent: created";
}

LmaoComponent::~LmaoComponent()
{
    qDebug() << "LmaoComponent: destroyed";
}

QString LmaoComponent::version() const
{
    char* raw = lmao_version();
    if (!raw)
        return QStringLiteral("unknown");
    QString v = QString::fromUtf8(raw);
    lmao_free_string(raw);
    return v;
}

void LmaoComponent::setupDeliveryTransport(LogosAPI* logosAPI)
{
    if (!logosAPI) {
        qWarning() << "LMAO: no LogosAPI — cannot set up QtRO delivery transport";
        return;
    }

    // Get a QtRO replica for delivery_module via the Logos Core API.
    // logosAPI->getClient() returns a QRemoteObjectDynamicReplica*.
    m_deliveryReplica = logosAPI->getClient(QStringLiteral("delivery_module"));
    if (!m_deliveryReplica) {
        qWarning() << "LMAO: failed to get delivery_module client";
        return;
    }

    // Wait for the replica to initialise (it connects asynchronously).
    if (!m_deliveryReplica->isInitialized()) {
        QObject::connect(m_deliveryReplica, &QRemoteObjectDynamicReplica::initialized,
            this, [this]() {
                qDebug() << "LMAO: delivery_module replica initialised";
                // Register FFI callbacks now that the replica is ready.
                lmao_qtro_set_callbacks(
                    qtro_publish, qtro_subscribe, qtro_unsubscribe,
                    static_cast<void*>(m_deliveryReplica));
            });
    } else {
        // Already initialised (e.g. local mode) — register immediately.
        lmao_qtro_set_callbacks(
            qtro_publish, qtro_subscribe, qtro_unsubscribe,
            static_cast<void*>(m_deliveryReplica));
    }

    // Forward inbound messages from the replica to Rust.
    QObject::connect(m_deliveryReplica, SIGNAL(messageReceived(QString)),
        this, SLOT(onDeliveryMessage(QString)));

    qDebug() << "LMAO: QtRO delivery transport bridge configured";
}

void LmaoComponent::initialize(LogosAPI* logosAPI)
{
    if (m_initialized)
        return;

    qDebug() << "LmaoComponent::initialize — starting LMAO node";

    // Wire up QtRO delivery transport before initialising the Rust node.
    setupDeliveryTransport(logosAPI);

    // Trigger lazy node init inside lmao-ffi by fetching the agent card.
    char* raw = lmao_get_agent_card();
    if (raw) {
        qDebug() << "LmaoComponent: node initialized, card:" << raw;
        lmao_free_string(raw);
    }

    m_initialized = true;
}

QWidget* LmaoComponent::createWidget(LogosAPI* logosAPI)
{
    initialize(logosAPI);

    auto* widget = new QQuickWidget();
    widget->setMinimumSize(500, 400);
    widget->setResizeMode(QQuickWidget::SizeRootObjectToView);

    auto* backend = new LmaoBackend();
    backend->setParent(widget);

    auto* model = new AgentListModel();
    model->setParent(widget);

    widget->rootContext()->setContextProperty("lmaoModule", backend);
    widget->rootContext()->setContextProperty("lmaoAgentModel", model);
    widget->setSource(QUrl("qrc:/lmao/LmaoView.qml"));

    return widget;
}

void LmaoComponent::destroyWidget(QWidget* widget)
{
    delete widget;
}

void LmaoComponent::onDeliveryMessage(const QString& eventJson)
{
    // Parse the event JSON to extract contentTopic and payload, then forward
    // to the Rust transport layer via the FFI bridge.
    QJsonDocument doc = QJsonDocument::fromJson(eventJson.toUtf8());
    if (!doc.isObject())
        return;

    QJsonObject obj = doc.object();
    QString topic = obj.value(QLatin1String("contentTopic")).toString();
    QString payload = obj.value(QLatin1String("payload")).toString();

    if (topic.isEmpty() || payload.isEmpty())
        return;

    const QByteArray topicUtf8 = topic.toUtf8();
    const QByteArray payloadUtf8 = payload.toUtf8();

    lmao_qtro_on_message(topicUtf8.constData(), payloadUtf8.constData());
}
