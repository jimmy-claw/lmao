#pragma once

#include <QObject>
#include <QString>
#include <QByteArray>

class DeliveryTransport;

/**
 * LmaoBackend — QObject wrapper around lmao-ffi for QML integration.
 *
 * Exposes agent discovery, task sending, and agent card retrieval
 * as Q_INVOKABLE methods callable from QML.
 *
 * When a DeliveryTransport is provided and connected, sendTask() uses the
 * real logos-co/logos-delivery-module via QtRO inter-module IPC (issue #143).
 */
class LmaoBackend : public QObject {
    Q_OBJECT

public:
    explicit LmaoBackend(DeliveryTransport* delivery = nullptr, QObject* parent = nullptr);
    ~LmaoBackend() override = default;

    Q_INVOKABLE QString discoverAgents(const QString& timeoutMs);
    Q_INVOKABLE QString sendTask(const QString& agentPubkey, const QString& taskText);
    Q_INVOKABLE QString getAgentCard();
    Q_INVOKABLE QString getInfo();
    Q_INVOKABLE QString getMetrics();

    /// Send raw payload to a content topic via QtRO delivery transport.
    Q_INVOKABLE bool deliverySend(const QString& contentTopic, const QByteArray& payload);

    /// Deprecated: sendTask() now automatically prefers QtRO delivery.
    Q_INVOKABLE QString sendTaskViaDelivery(const QString& agentPubkey, const QString& taskText);

signals:
    void agentsDiscovered(const QString& json);
    void taskSent(const QString& json);
    void errorOccurred(const QString& error);

private:
    static QString callFfiStr(char* raw);
    DeliveryTransport* m_delivery = nullptr;
};
