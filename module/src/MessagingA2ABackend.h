#pragma once

#include <QObject>
#include <QString>
#include <QStringList>
#include <QJsonArray>

extern "C" {
    void waku_a2a_free_string(char *s);
    int waku_a2a_init(const char *name, const char *description,
                      const char *nwaku_url, bool encrypted);
    char *waku_a2a_pubkey(void);
    char *waku_a2a_agent_card_json(void);
    int waku_a2a_announce(void);
    char *waku_a2a_discover(void);
    int waku_a2a_send_text(const char *to_pubkey, const char *text);
    char *waku_a2a_poll_tasks(void);
    int waku_a2a_respond(const char *task_json, const char *result_text);
    void waku_a2a_shutdown(void);
}

class MessagingA2ABackend : public QObject {
    Q_OBJECT
    Q_PROPERTY(QString pubkey READ pubkey NOTIFY initialized)
    Q_PROPERTY(QString agentCard READ agentCard NOTIFY initialized)
    Q_PROPERTY(bool ready READ ready NOTIFY initialized)
    Q_PROPERTY(QJsonArray agents READ agents NOTIFY agentsChanged)
    Q_PROPERTY(QJsonArray tasks READ tasks NOTIFY tasksChanged)

public:
    explicit MessagingA2ABackend(QObject *parent = nullptr);
    ~MessagingA2ABackend() override;

    QString pubkey() const { return m_pubkey; }
    QString agentCard() const { return m_agentCard; }
    bool ready() const { return m_ready; }
    QJsonArray agents() const { return m_agents; }
    QJsonArray tasks() const { return m_tasks; }

    Q_INVOKABLE bool initialize(const QString &name, const QString &description,
                                 const QString &nwakuUrl, bool encrypted);
    Q_INVOKABLE bool announce();
    Q_INVOKABLE void discover();
    Q_INVOKABLE bool sendText(const QString &toPubkey, const QString &text);
    Q_INVOKABLE void pollTasks();
    Q_INVOKABLE bool respond(const QString &taskJson, const QString &resultText);
    Q_INVOKABLE void shutdown();

signals:
    void initialized();
    void agentsChanged();
    void tasksChanged();
    void messageSent(const QString &toPubkey);
    void errorOccurred(const QString &error);

private:
    QString ffiString(char *raw);
    bool m_ready = false;
    QString m_pubkey;
    QString m_agentCard;
    QJsonArray m_agents;
    QJsonArray m_tasks;
};
