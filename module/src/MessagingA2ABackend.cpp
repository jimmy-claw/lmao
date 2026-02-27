#include "MessagingA2ABackend.h"
#include <QJsonDocument>
#include <QJsonArray>
#include <QByteArray>
#include <cstring>

MessagingA2ABackend::MessagingA2ABackend(QObject *parent)
    : QObject(parent) {}

MessagingA2ABackend::~MessagingA2ABackend() {
    if (m_ready) {
        waku_a2a_shutdown();
    }
}

QString MessagingA2ABackend::ffiString(char *raw) {
    if (!raw) return {};
    QString result = QString::fromUtf8(raw);
    waku_a2a_free_string(raw);
    return result;
}

bool MessagingA2ABackend::initialize(const QString &name, const QString &description,
                                      const QString &nwakuUrl, bool encrypted) {
    int rc = waku_a2a_init(name.toUtf8().constData(),
                           description.toUtf8().constData(),
                           nwakuUrl.toUtf8().constData(),
                           encrypted);
    if (rc != 0) {
        emit errorOccurred("Failed to initialize A2A node");
        return false;
    }

    m_pubkey = ffiString(waku_a2a_pubkey());
    m_agentCard = ffiString(waku_a2a_agent_card_json());
    m_ready = true;
    emit initialized();
    return true;
}

bool MessagingA2ABackend::announce() {
    if (!m_ready) return false;
    int rc = waku_a2a_announce();
    if (rc != 0) {
        emit errorOccurred("Announce failed");
        return false;
    }
    return true;
}

void MessagingA2ABackend::discover() {
    if (!m_ready) return;
    QString json = ffiString(waku_a2a_discover());
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    if (doc.isArray()) {
        m_agents = doc.array();
        emit agentsChanged();
    }
}

bool MessagingA2ABackend::sendText(const QString &toPubkey, const QString &text) {
    if (!m_ready) return false;
    int rc = waku_a2a_send_text(toPubkey.toUtf8().constData(),
                                 text.toUtf8().constData());
    if (rc != 0) {
        emit errorOccurred("Send failed");
        return false;
    }
    emit messageSent(toPubkey);
    return true;
}

void MessagingA2ABackend::pollTasks() {
    if (!m_ready) return;
    QString json = ffiString(waku_a2a_poll_tasks());
    QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    if (doc.isArray()) {
        m_tasks = doc.array();
        emit tasksChanged();
    }
}

bool MessagingA2ABackend::respond(const QString &taskJson, const QString &resultText) {
    if (!m_ready) return false;
    int rc = waku_a2a_respond(taskJson.toUtf8().constData(),
                               resultText.toUtf8().constData());
    if (rc != 0) {
        emit errorOccurred("Respond failed");
        return false;
    }
    return true;
}

void MessagingA2ABackend::shutdown() {
    if (m_ready) {
        waku_a2a_shutdown();
        m_ready = false;
    }
}
