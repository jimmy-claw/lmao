#pragma once

#include <QObject>
#include <QString>

/**
 * LmaoBackend — QObject wrapper around lmao-ffi for QML integration.
 *
 * Exposes agent discovery, task sending, and agent card retrieval
 * as Q_INVOKABLE methods callable from QML.
 */
class LmaoBackend : public QObject {
    Q_OBJECT

public:
    explicit LmaoBackend(QObject* parent = nullptr);
    ~LmaoBackend() override = default;

    Q_INVOKABLE QString discoverAgents(const QString& timeoutMs);
    Q_INVOKABLE QString sendTask(const QString& agentPubkey, const QString& taskText);
    Q_INVOKABLE QString getAgentCard();

signals:
    void agentsDiscovered(const QString& json);
    void taskSent(const QString& json);
    void errorOccurred(const QString& error);

private:
    static QString callFfiStr(char* raw);
};
