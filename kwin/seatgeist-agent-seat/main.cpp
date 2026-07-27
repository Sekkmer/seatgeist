// SPDX-License-Identifier: MIT OR Apache-2.0

#include "agentseatplugin.h"
#include "kwin_plugin_abi.h"

#include <plugin.h>

class KWIN_EXPORT SeatgeistAgentSeatFactory final : public KWin::PluginFactory
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID SEATGEIST_KWIN_PLUGIN_FACTORY_IID FILE "metadata.json")
    Q_INTERFACES(KWin::PluginFactory)

public:
    std::unique_ptr<KWin::Plugin> create() const override
    {
        return std::make_unique<KWin::SeatgeistAgentSeatPlugin>();
    }
};

#include "main.moc"
