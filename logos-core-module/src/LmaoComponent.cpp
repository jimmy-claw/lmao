#include "LmaoComponent.h"
#include "LmaoBackend.h"
#include "AgentListModel.h"

#include <QQuickWidget>
#include <QQmlContext>
#include <QDebug>

#ifdef __cplusplus
extern "C" {
#endif
#include "lmao_ffi.h"
#ifdef __cplusplus
}
#endif

// ---------------------------------------------------------------------------
// Delivery transport now uses logos_core_call_plugin_method_async directly
// from Rust — no C++ callback injection needed. The Rust transport calls
// the delivery_module plugin methods through the Logos Core C IPC layer.
// ---------------------------------------------------------------------------

LmaoComponent::LmaoComponent(QObject* parent)
    : QObject(parent)
{
    qDebug() << "LmaoComponent: created";
}

LmaoComponent::~LmaoComponent()
{
    qDebug() << "LmaoComponent: destroyed";
}

QString LmaoComponent::version() const
{
    char* raw = lmao_version();
    if (!raw)
        return QStringLiteral("unknown");
    QString v = QString::fromUtf8(raw);
    lmao_free_string(raw);
    return v;
}

void LmaoComponent::initialize(LogosAPI* logosAPI)
{
    if (m_initialized)
        return;

    qDebug() << "LmaoComponent::initialize — starting LMAO node";

    // Delivery transport now uses logos_core_call_plugin_method_async directly
    // from Rust — no C++ callback setup needed.

    // Trigger lazy node init inside lmao-ffi by fetching the agent card.
    char* raw = lmao_get_agent_card();
    if (raw) {
        qDebug() << "LmaoComponent: node initialized, card:" << raw;
        lmao_free_string(raw);
    }

    m_initialized = true;
}

QWidget* LmaoComponent::createWidget(LogosAPI* logosAPI)
{
    initialize(logosAPI);

    auto* widget = new QQuickWidget();
    widget->setMinimumSize(500, 400);
    widget->setResizeMode(QQuickWidget::SizeRootObjectToView);

    auto* backend = new LmaoBackend();
    backend->setParent(widget);

    auto* model = new AgentListModel();
    model->setParent(widget);

    widget->rootContext()->setContextProperty("lmaoModule", backend);
    widget->rootContext()->setContextProperty("lmaoAgentModel", model);
    widget->setSource(QUrl("qrc:/lmao/LmaoView.qml"));

    return widget;
}

void LmaoComponent::destroyWidget(QWidget* widget)
{
    delete widget;
}

// onDeliveryMessage removed — inbound messages from delivery_module are now
// handled directly by the Rust transport via logos_core_register_event_listener.
