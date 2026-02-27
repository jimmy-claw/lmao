#pragma once

#include <IComponent.h>
#include <QObject>

class MessagingA2AUIComponent : public QObject, public IComponent {
    Q_OBJECT
    Q_INTERFACES(IComponent)
    Q_PLUGIN_METADATA(IID IComponent_iid FILE MESSAGING_A2A_UI_METADATA_FILE)

public:
    QWidget* createWidget(LogosAPI* logosAPI = nullptr) override;
    void destroyWidget(QWidget* widget) override;
};
