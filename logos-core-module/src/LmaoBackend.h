#pragma once

#include <QObject>
#include <QString>

class DeliveryTransport;

/**
 * LmaoBackend — QObject wrapper around lmao-ffi for QML integration.
 *
 * Exposes agent discovery, task sending, and agent card retrieval
 * as Q_INVOKABLE methods callable from QML.
 *
 * When a DeliveryTransport (QtRO) is available, all message sending
 * automatically uses the real logos-delivery-module via QtRO inter-module
 * calls (issue #77, #143). Falls back to Rust FFI transport when QtRO
 * is unavailable.
 */
class LmaoBackend : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool hasDeliveryTransport READ hasDeliveryTransport CONSTANT)

public:
    explicit LmaoBackend(DeliveryTransport* delivery = nullptr,
                         QObject* parent = nullptr);
    ~LmaoBackend() override = default;

    bool hasDeliveryTransport() const { return m_delivery != nullptr; }

    Q_INVOKABLE QString discoverAgents(const QString& timeoutMs);
    Q_INVOKABLE QString sendTask(const QString& agentPubkey, const QString& taskText);
    Q_INVOKABLE QString getAgentCard();
    Q_INVOKABLE QString getInfo();
    Q_INVOKABLE QString getMetrics();

    /// Send raw bytes to a content topic via QtRO delivery transport.
    /// Returns true if sent via QtRO, false if QtRO unavailable (caller should fall back).
    Q_INVOKABLE bool deliverySend(const QString& contentTopic, const QByteArray& payload);

    /// Deprecated: sendTask() now automatically uses QtRO delivery when available.
    /// Kept for backwards compatibility with existing QML code.
    Q_INVOKABLE QString sendTaskViaDelivery(const QString& agentPubkey, const QString& taskText);

signals:
    void agentsDiscovered(const QString& json);
    void taskSent(const QString& json);
    void errorOccurred(const QString& error);

private:
    static QString callFfiStr(char* raw);
    DeliveryTransport* m_delivery = nullptr;
};
