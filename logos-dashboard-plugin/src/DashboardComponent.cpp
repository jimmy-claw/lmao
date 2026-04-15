#include "DashboardComponent.h"
#include "DashboardBackend.h"
#include "lmao_ffi.h"

#include <QQuickWidget>
#include <QQmlContext>
#include <QDebug>

DashboardComponent::DashboardComponent(QObject *parent)
    : QObject(parent)
{
    qDebug() << "[LmaoDashboard] Component created";
}

DashboardComponent::~DashboardComponent()
{
    qDebug() << "[LmaoDashboard] Component destroyed";
}

QString DashboardComponent::version() const
{
    char *v = lmao_version();
    if (!v) return QStringLiteral("unknown");
    QString ver = QString::fromUtf8(v);
    lmao_free_string(v);
    return ver;
}

void DashboardComponent::initialize()
{
    if (m_initialized) return;
    // Trigger lazy node initialization
    char *card = lmao_get_agent_card();
    if (card) lmao_free_string(card);
    m_initialized = true;
    qDebug() << "[LmaoDashboard] Initialized";
}

QWidget *DashboardComponent::createWidget(LogosAPI * /*api*/)
{
    initialize();

    auto *widget = new QQuickWidget;
    widget->setResizeMode(QQuickWidget::SizeRootObjectToView);

    auto *backend = new DashboardBackend(widget);
    widget->rootContext()->setContextProperty(
        QStringLiteral("dashboardBackend"), backend);

    widget->setSource(QUrl(QStringLiteral("qrc:/lmao-dashboard/DashboardView.qml")));

    if (widget->status() == QQuickWidget::Error) {
        qWarning() << "[LmaoDashboard] QML errors:" << widget->errors();
    }

    return widget;
}

void DashboardComponent::destroyWidget(QWidget *widget)
{
    delete widget;
}
