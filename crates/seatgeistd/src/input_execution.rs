use std::fmt::Display;

use anyhow::Result;
use libseatgeist::{Point, PointerButton};

#[cfg(test)]
use crate::portal_eis_session::DaemonPortalEisSession;
use crate::{
    config::InputBackendPreference, eis_key_combo, keymap::Settings as XkbKeymapSettings,
    portal_eis_session::PortalEisSessionStore,
};

const EIS_PLAN_SEQUENCE: u32 = 1;

pub(crate) trait InputExecutionBackend {
    fn name(&self) -> &'static str;
    fn type_text(&mut self, text: &str) -> Result<()>;
    fn key_combo(&mut self, combo: &str) -> Result<usize>;
    fn move_pointer(&mut self, point: Point, bounds: seatgeist_uinput::PointerBounds)
    -> Result<()>;
    fn click_pointer(
        &mut self,
        point: Point,
        bounds: seatgeist_uinput::PointerBounds,
        button: PointerButton,
        clicks: u8,
    ) -> Result<()>;
    fn drag_pointer(
        &mut self,
        from: Point,
        to: Point,
        bounds: seatgeist_uinput::PointerBounds,
        button: PointerButton,
        duration_ms: u64,
    ) -> Result<()>;
    fn scroll_pointer(
        &mut self,
        vertical: i32,
        horizontal: i32,
        bounds: seatgeist_uinput::PointerBounds,
    ) -> Result<()>;
}

struct UinputInputExecutionBackend;

impl InputExecutionBackend for UinputInputExecutionBackend {
    fn name(&self) -> &'static str {
        "uinput"
    }

    fn type_text(&mut self, text: &str) -> Result<()> {
        seatgeist_uinput::type_text(text).map_err(|err| anyhow::anyhow!(err))
    }

    fn key_combo(&mut self, combo: &str) -> Result<usize> {
        seatgeist_uinput::key_combo(combo).map_err(|err| anyhow::anyhow!(err))
    }

    fn move_pointer(
        &mut self,
        point: Point,
        bounds: seatgeist_uinput::PointerBounds,
    ) -> Result<()> {
        seatgeist_uinput::move_pointer(point.x, point.y, bounds).map_err(|err| anyhow::anyhow!(err))
    }

    fn click_pointer(
        &mut self,
        point: Point,
        bounds: seatgeist_uinput::PointerBounds,
        button: PointerButton,
        clicks: u8,
    ) -> Result<()> {
        seatgeist_uinput::click_pointer(
            point.x,
            point.y,
            bounds,
            pointer_button_to_uinput(button),
            clicks,
        )
        .map_err(|err| anyhow::anyhow!(err))
    }

    fn drag_pointer(
        &mut self,
        from: Point,
        to: Point,
        bounds: seatgeist_uinput::PointerBounds,
        button: PointerButton,
        duration_ms: u64,
    ) -> Result<()> {
        seatgeist_uinput::drag_pointer(
            from.x,
            from.y,
            to.x,
            to.y,
            bounds,
            pointer_button_to_uinput(button),
            duration_ms,
        )
        .map_err(|err| anyhow::anyhow!(err))
    }

    fn scroll_pointer(
        &mut self,
        vertical: i32,
        horizontal: i32,
        bounds: seatgeist_uinput::PointerBounds,
    ) -> Result<()> {
        seatgeist_uinput::scroll_pointer(vertical, horizontal, bounds)
            .map_err(|err| anyhow::anyhow!(err))
    }
}

trait EisPlanExecutor {
    fn backend_name(&self) -> &'static str;
    fn execute_plan(&mut self, plan: &seatgeist_eis::EisActionPlan) -> Result<()>;
}

struct StoredEisPlanExecutor<'a, S> {
    backend_name: &'static str,
    store: &'a PortalEisSessionStore<S>,
}

impl<S> EisPlanExecutor for StoredEisPlanExecutor<'_, S>
where
    S: seatgeist_eis::EisEventSource + seatgeist_eis::EisSelectedDeviceExecutor,
    S::Error: Display,
{
    fn backend_name(&self) -> &'static str {
        self.backend_name
    }

    fn execute_plan(&mut self, plan: &seatgeist_eis::EisActionPlan) -> Result<()> {
        self.store.execute_ready_plan(self.backend_name, plan)
    }
}

struct EisInputExecutionBackend<E> {
    executor: E,
    keymap_settings: XkbKeymapSettings,
}

impl<E: EisPlanExecutor> EisInputExecutionBackend<E> {
    fn execute_plan(&mut self, plan: seatgeist_eis::EisActionPlan) -> Result<()> {
        self.executor.execute_plan(&plan)
    }
}

impl<E: EisPlanExecutor> InputExecutionBackend for EisInputExecutionBackend<E> {
    fn name(&self) -> &'static str {
        self.executor.backend_name()
    }

    fn type_text(&mut self, text: &str) -> Result<()> {
        let plan = seatgeist_eis::plan_text_utf8(EIS_PLAN_SEQUENCE, text)
            .map_err(|err| anyhow::anyhow!(err))?;
        self.execute_plan(plan)
    }

    fn key_combo(&mut self, combo: &str) -> Result<usize> {
        let codes = eis_key_combo::codes(combo, &self.keymap_settings)?;
        let plan = seatgeist_eis::plan_key_combo_evdev(EIS_PLAN_SEQUENCE, &codes)
            .map_err(|err| anyhow::anyhow!(err))?;
        let key_count = codes.len();
        self.execute_plan(plan)?;
        Ok(key_count)
    }

    fn move_pointer(
        &mut self,
        point: Point,
        _bounds: seatgeist_uinput::PointerBounds,
    ) -> Result<()> {
        self.execute_plan(seatgeist_eis::plan_pointer_move_absolute(
            EIS_PLAN_SEQUENCE,
            point,
        ))
    }

    fn click_pointer(
        &mut self,
        point: Point,
        _bounds: seatgeist_uinput::PointerBounds,
        button: PointerButton,
        clicks: u8,
    ) -> Result<()> {
        let plan =
            seatgeist_eis::plan_pointer_click_absolute(EIS_PLAN_SEQUENCE, point, button, clicks)
                .map_err(|err| anyhow::anyhow!(err))?;
        self.execute_plan(plan)
    }

    fn drag_pointer(
        &mut self,
        from: Point,
        to: Point,
        _bounds: seatgeist_uinput::PointerBounds,
        button: PointerButton,
        _duration_ms: u64,
    ) -> Result<()> {
        self.execute_plan(seatgeist_eis::plan_pointer_drag_absolute(
            EIS_PLAN_SEQUENCE,
            from,
            to,
            button,
        ))
    }

    fn scroll_pointer(
        &mut self,
        vertical: i32,
        horizontal: i32,
        _bounds: seatgeist_uinput::PointerBounds,
    ) -> Result<()> {
        let plan =
            seatgeist_eis::plan_pointer_scroll_discrete(EIS_PLAN_SEQUENCE, vertical, horizontal)
                .map_err(|err| anyhow::anyhow!(err))?;
        self.execute_plan(plan)
    }
}

pub(crate) fn backend(
    preference: InputBackendPreference,
    portal_eis_session_store: &PortalEisSessionStore,
) -> Result<Box<dyn InputExecutionBackend + '_>> {
    backend_with_store(
        preference,
        portal_eis_session_store,
        &XkbKeymapSettings::default(),
    )
}

pub(crate) fn backend_with_store<'a, S>(
    preference: InputBackendPreference,
    portal_eis_session_store: &'a PortalEisSessionStore<S>,
    keymap_settings: &XkbKeymapSettings,
) -> Result<Box<dyn InputExecutionBackend + 'a>>
where
    S: seatgeist_eis::EisEventSource + seatgeist_eis::EisSelectedDeviceExecutor + 'a,
    S::Error: Display,
{
    match preference {
        InputBackendPreference::Auto | InputBackendPreference::Uinput => {
            Ok(Box::new(UinputInputExecutionBackend))
        }
        InputBackendPreference::PortalRemoteDesktop => Ok(Box::new(EisInputExecutionBackend {
            executor: StoredEisPlanExecutor {
                backend_name: "portal_remote_desktop",
                store: portal_eis_session_store,
            },
            keymap_settings: keymap_settings.clone(),
        })),
        InputBackendPreference::Libei => Ok(Box::new(EisInputExecutionBackend {
            executor: StoredEisPlanExecutor {
                backend_name: "libei",
                store: portal_eis_session_store,
            },
            keymap_settings: keymap_settings.clone(),
        })),
    }
}

fn pointer_button_to_uinput(button: PointerButton) -> seatgeist_uinput::PointerButton {
    match button {
        PointerButton::Left => seatgeist_uinput::PointerButton::Left,
        PointerButton::Middle => seatgeist_uinput::PointerButton::Middle,
        PointerButton::Right => seatgeist_uinput::PointerButton::Right,
    }
}

#[cfg(test)]
pub(crate) fn session_backend<'a, S>(
    backend_name: &'static str,
    session: &'a mut DaemonPortalEisSession<S>,
) -> Box<dyn InputExecutionBackend + 'a>
where
    S: seatgeist_eis::EisEventSource + seatgeist_eis::EisSelectedDeviceExecutor + 'a,
    S::Error: Display,
{
    Box::new(EisInputExecutionBackend {
        executor: SessionEisPlanExecutor {
            backend_name,
            session,
        },
        keymap_settings: XkbKeymapSettings::default(),
    })
}

#[cfg(test)]
struct SessionEisPlanExecutor<'a, S> {
    backend_name: &'static str,
    session: &'a mut DaemonPortalEisSession<S>,
}

#[cfg(test)]
impl<S> EisPlanExecutor for SessionEisPlanExecutor<'_, S>
where
    S: seatgeist_eis::EisEventSource + seatgeist_eis::EisSelectedDeviceExecutor,
    S::Error: Display,
{
    fn backend_name(&self) -> &'static str {
        self.backend_name
    }

    fn execute_plan(&mut self, plan: &seatgeist_eis::EisActionPlan) -> Result<()> {
        self.session.execute_ready_plan(plan)?;
        Ok(())
    }
}
