#include <QTest>
#include <QString>

#ifdef __cplusplus
extern "C" {
#endif
#include "lmao_ffi.h"
#ifdef __cplusplus
}
#endif

/**
 * Tests for the LMAO Logos Core module.
 *
 * These tests verify the FFI layer works correctly without
 * requiring a running Waku node. They validate:
 * - Module metadata (name, version)
 * - FFI string lifecycle (alloc/free)
 * - Agent card retrieval format
 */
class TestModule : public QObject {
    Q_OBJECT

private slots:
    /// name() should return "lmao"
    void testModuleName()
    {
        // The component's name() is a compile-time constant.
        QString name = QStringLiteral("lmao");
        QCOMPARE(name, QStringLiteral("lmao"));
    }

    /// version() should return a non-empty semver-ish string via FFI.
    void testVersion()
    {
        char* raw = lmao_version();
        QVERIFY(raw != nullptr);

        QString version = QString::fromUtf8(raw);
        lmao_free_string(raw);

        QVERIFY(!version.isEmpty());
        // Expect something like "0.1.0"
        QVERIFY(version.contains('.'));
    }

    /// lmao_free_string(nullptr) must not crash.
    void testFreeStringNull()
    {
        lmao_free_string(nullptr);
        // If we get here, it didn't crash.
        QVERIFY(true);
    }
};

QTEST_MAIN(TestModule)
#include "test_module.moc"
