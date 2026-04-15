#ifndef ICOMPONENT_H
#define ICOMPONENT_H

#include <QWidget>
#include <QtPlugin>

class LogosAPI;

class IComponent {
public:
    virtual ~IComponent() = default;
    virtual QWidget *createWidget(LogosAPI *api = nullptr) = 0;
    virtual void destroyWidget(QWidget *widget) = 0;
};

Q_DECLARE_INTERFACE(IComponent, "com.logos.component.IComponent")

#endif // ICOMPONENT_H
