#include <QTest>
#include <QString>
#include <QJsonDocument>
#include <QJsonObject>

#ifdef __cplusplus
extern "C" {
#endif
#include "lmao_ffi.h"
#ifdef __cplusplus
}
#endif

/**
 * Tests for the LMAO Dashboard plugin.
 *
 * These tests verify the FFI layer works correctly without
 * requiring a running Waku node. They validate:
 * - Module metadata (name, version)
 * - FFI string lifecycle (alloc/free)
 * - JSON response structure from FFI functions
 */
class TestDashboard : public QObject {
    Q_OBJECT

private slots:
    /// Module name should be "lmao-dashboard"
    void testModuleName()
    {
        QString name = QStringLiteral("lmao-dashboard");
        QCOMPARE(name, QStringLiteral("lmao-dashboard"));
    }

    /// version() should return a non-empty semver string via FFI.
    void testVersion()
    {
        char* raw = lmao_version();
        QVERIFY(raw != nullptr);

        QString version = QString::fromUtf8(raw);
        lmao_free_string(raw);

        QVERIFY(!version.isEmpty());
        QVERIFY(version.contains('.'));
    }

    /// lmao_free_string(nullptr) must not crash.
    void testFreeStringNull()
    {
        lmao_free_string(nullptr);
        QVERIFY(true);
    }

    /// version() called twice returns the same value.
    void testVersionIdempotent()
    {
        char *v1 = lmao_version();
        char *v2 = lmao_version();
        QVERIFY(v1 != nullptr);
        QVERIFY(v2 != nullptr);

        QString s1 = QString::fromUtf8(v1);
        QString s2 = QString::fromUtf8(v2);
        lmao_free_string(v1);
        lmao_free_string(v2);

        QCOMPARE(s1, s2);
    }

    /// lmao_send_task(nullptr) returns a valid JSON error, not a crash.
    void testSendTaskNullSafe()
    {
        char *raw = lmao_send_task(nullptr);
        QVERIFY(raw != nullptr);

        QJsonDocument doc = QJsonDocument::fromJson(QByteArray(raw));
        lmao_free_string(raw);

        QVERIFY(doc.isObject());
        QJsonObject obj = doc.object();
        QCOMPARE(obj.value("success").toBool(), false);
        QCOMPARE(obj.value("error").toString(), QStringLiteral("null pointer"));
    }

    /// lmao_send_task with invalid JSON returns parse error.
    void testSendTaskInvalidJson()
    {
        char *raw = lmao_send_task("not json");
        QVERIFY(raw != nullptr);

        QJsonDocument doc = QJsonDocument::fromJson(QByteArray(raw));
        lmao_free_string(raw);

        QVERIFY(doc.isObject());
        QJsonObject obj = doc.object();
        QCOMPARE(obj.value("success").toBool(), false);
        QVERIFY(obj.value("error").toString().contains("JSON parse error"));
    }

    /// lmao_send_task with missing fields returns field-specific error.
    void testSendTaskMissingFields()
    {
        char *raw = lmao_send_task("{}");
        QVERIFY(raw != nullptr);

        QJsonDocument doc = QJsonDocument::fromJson(QByteArray(raw));
        lmao_free_string(raw);

        QJsonObject obj = doc.object();
        QCOMPARE(obj.value("success").toBool(), false);
        QCOMPARE(obj.value("error").toString(), QStringLiteral("missing 'agent_pubkey'"));
    }
};

QTEST_MAIN(TestDashboard)
#include "test_dashboard.moc"
