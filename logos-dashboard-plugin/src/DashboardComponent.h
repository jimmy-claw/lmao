#ifndef DASHBOARD_COMPONENT_H
#define DASHBOARD_COMPONENT_H

#include <QObject>
#include <QWidget>
#include <QtPlugin>

#include "IComponent.h"

class DashboardComponent : public QObject, public IComponent {
    Q_OBJECT
    Q_INTERFACES(IComponent)
    Q_PLUGIN_METADATA(IID "com.logos.component.IComponent" FILE "metadata.json")

public:
    explicit DashboardComponent(QObject *parent = nullptr);
    ~DashboardComponent() override;

    QWidget *createWidget(LogosAPI *api = nullptr) override;
    void destroyWidget(QWidget *widget) override;

    QString name() const { return QStringLiteral("lmao-dashboard"); }
    QString version() const;
    void initialize();

private:
    bool m_initialized = false;
};

#endif // DASHBOARD_COMPONENT_H
