#pragma once

#include <IComponent.h>
#include <QObject>
#include <QString>

class LmaoBackend;
class QRemoteObjectDynamicReplica;

/**
 * LmaoComponent — Logos Core IComponent plugin for LMAO (A2A over Waku).
 *
 * Provides agent discovery and task sending over the Waku network.
 * Connects to delivery_module via QtRO for message transport.
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

    /// Initialize the LMAO node and wire up QtRO delivery transport.
    void initialize(LogosAPI* logosAPI = nullptr);

private slots:
    /// Forward inbound delivery_module messages to the Rust transport layer.
    void onDeliveryMessage(const QString& eventJson);

private:
    bool m_initialized = false;
    /// QtRO replica for delivery_module — kept alive for the component lifetime.
    QRemoteObjectDynamicReplica* m_deliveryReplica = nullptr;

    /// Set up the QtRO delivery transport bridge to Rust via FFI callbacks.
    void setupDeliveryTransport(LogosAPI* logosAPI);
};
