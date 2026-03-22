#pragma once

#include <IComponent.h>
#include <QObject>
#include <QString>

class LmaoBackend;

/**
 * LmaoComponent — Logos Core IComponent plugin for LMAO (A2A over Waku).
 *
 * Provides agent discovery and task sending over the Waku network.
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

    /// Initialize the LMAO node (calls into FFI).
    void initialize();

private:
    bool m_initialized = false;
};
