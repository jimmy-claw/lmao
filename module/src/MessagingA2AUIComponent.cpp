#include "MessagingA2AUIComponent.h"
#include "MessagingA2ABackend.h"

#include <QQuickWidget>
#include <QQmlContext>

QWidget* MessagingA2AUIComponent::createWidget(LogosAPI* /*logosAPI*/) {
    auto* quickWidget = new QQuickWidget();
    quickWidget->setMinimumSize(500, 400);
    quickWidget->setResizeMode(QQuickWidget::SizeRootObjectToView);

    auto* backend = new MessagingA2ABackend();
    backend->setParent(quickWidget);

    quickWidget->rootContext()->setContextProperty("a2aBackend", backend);
    quickWidget->setSource(QUrl("qrc:/messaging-a2a/MessagingA2AView.qml"));

    return quickWidget;
}

void MessagingA2AUIComponent::destroyWidget(QWidget* widget) {
    delete widget;
}
