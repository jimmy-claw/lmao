#pragma once

#include <IComponent.h>
#include <QObject>
#include <QString>

class LmaoBackend;

/**
 * LmaoComponent — Logos Core IComponent plugin for LMAO (A2A over Waku).
 *
 * Provides agent discovery and task sending over the Waku network.
 * Delivery transport uses logos_core_call_plugin_method_async directly
 * from Rust to call the delivery_module plugin via Logos Core IPC.
 */
class LmaoComponent : public QObject, public IComponent {
    Q_OBJECT
    Q_INTERFACES(IComponent)
    Q_PLUGIN_METADATA(IID IComponent_iid FILE "metadata.json")

public:
    explicit LmaoComponent(QObject* parent = nullptr);
    ~LmaoComponent() override;

    QWidget* createWidget(LogosAPI* logosAPI = nullptr) override;
    void destroyWidget(QWidget* widget) override;

    QString name() const { return QStringLiteral("lmao"); }
    QString version() const;

    /// Initialize the LMAO node. Delivery transport is handled by the Rust
    /// layer via logos_core_call_plugin_method_async.
    void initialize(LogosAPI* logosAPI = nullptr);

private:
    bool m_initialized = false;
};
