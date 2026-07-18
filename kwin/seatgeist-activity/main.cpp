// SPDX-License-Identifier: MIT OR Apache-2.0

#include "activityplugin.h"
#include "kwin_plugin_abi.h"

#include <plugin.h>

class KWIN_EXPORT SeatgeistActivityFactory final : public KWin::PluginFactory
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID PluginFactory_iid FILE "metadata.json")
    Q_INTERFACES(KWin::PluginFactory)

public:
    std::unique_ptr<KWin::Plugin> create() const override
    {
        return std::make_unique<KWin::SeatgeistActivityPlugin>();
    }
};

#include "main.moc"
