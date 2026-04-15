#ifndef DASHBOARD_BACKEND_H
#define DASHBOARD_BACKEND_H

#include <QObject>
#include <QString>
#include <QJsonObject>
#include <QJsonArray>

#ifdef __cplusplus
extern "C" {
#endif
#include "lmao_ffi.h"
#ifdef __cplusplus
}
#endif

class DashboardBackend : public QObject {
    Q_OBJECT

    Q_PROPERTY(QString publicKey READ publicKey NOTIFY infoChanged)
    Q_PROPERTY(QString taskTopic READ taskTopic NOTIFY infoChanged)
    Q_PROPERTY(QString discoveryTopic READ discoveryTopic NOTIFY infoChanged)
    Q_PROPERTY(QString presenceTopic READ presenceTopic NOTIFY infoChanged)
    Q_PROPERTY(bool encryption READ encryption NOTIFY infoChanged)
    Q_PROPERTY(QString version READ version CONSTANT)
    Q_PROPERTY(int peersDiscovered READ peersDiscovered NOTIFY metricsChanged)

public:
    explicit DashboardBackend(QObject *parent = nullptr);
    ~DashboardBackend() override = default;

    QString publicKey() const { return m_publicKey; }
    QString taskTopic() const { return m_taskTopic; }
    QString discoveryTopic() const { return m_discoveryTopic; }
    QString presenceTopic() const { return m_presenceTopic; }
    bool encryption() const { return m_encryption; }
    QString version() const { return m_version; }
    int peersDiscovered() const { return m_peersDiscovered; }

    Q_INVOKABLE void refreshInfo();
    Q_INVOKABLE void refreshMetrics();
    Q_INVOKABLE QJsonObject getMetrics();
    Q_INVOKABLE QString getAgentCardJson();

signals:
    void infoChanged();
    void metricsChanged(const QJsonObject &metrics);
    void errorOccurred(const QString &error);

private:
    static QString callFfiStr(char *raw);

    QString m_publicKey;
    QString m_taskTopic;
    QString m_discoveryTopic;
    QString m_presenceTopic;
    bool m_encryption = false;
    QString m_version;
    int m_peersDiscovered = 0;
};

#endif // DASHBOARD_BACKEND_H
