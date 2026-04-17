#pragma once

#include <QObject>
#include <QString>
#include <QByteArray>

class LogosAPI;

/**
 * DeliveryTransport — QtRO bridge to the delivery_module Logos Core module.
 *
 * Obtains a QtRO replica of delivery_module via logosAPI->getClient() and
 * exposes send/subscribe/unsubscribe as Q_INVOKABLE methods.
 *
 * This follows the same pattern used by logos-kv-module and lez-multisig-module.
 */
class DeliveryTransport : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool connected READ isConnected NOTIFY connectedChanged)

public:
    explicit DeliveryTransport(QObject* parent = nullptr);
    ~DeliveryTransport() override = default;

    /// Acquire the delivery_module replica from logosAPI.
    /// Returns true if the client was obtained successfully.
    bool init(LogosAPI* logosAPI);

    bool isConnected() const { return m_client != nullptr; }

    /// Send a payload to a content topic via delivery_module.
    Q_INVOKABLE bool send(const QString& contentTopic, const QByteArray& payload);

    /// Subscribe to a content topic.
    Q_INVOKABLE bool subscribe(const QString& contentTopic);

    /// Unsubscribe from a content topic.
    Q_INVOKABLE bool unsubscribe(const QString& contentTopic);

    /// Initialize the delivery node with a configuration JSON string.
    Q_INVOKABLE bool createNode(const QString& configJson);

    /// Start the delivery node.
    Q_INVOKABLE bool start();

signals:
    void connectedChanged();
    void messageReceived(const QString& contentTopic, const QByteArray& payload);

private slots:
    void onDeliveryEvent(const QString& eventName, const QString& eventData);

private:
    /// Call a method on the delivery_module replica via QMetaObject::invokeMethod.
    /// Returns the result string, or an empty string on failure.
    QString callMethod(const QString& method, const QVariantList& args);

    QObject* m_client = nullptr;
};
