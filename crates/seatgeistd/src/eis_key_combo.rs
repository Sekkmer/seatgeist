use anyhow::{Context, Result, bail};

use crate::keymap::Settings as XkbKeymapSettings;

pub(crate) fn codes(combo: &str, keymap_settings: &XkbKeymapSettings) -> Result<Vec<u16>> {
    let keymap = seatgeist_eis::XkbKeymap::new_from_names(keymap_settings.as_names())
        .map_err(|err| anyhow::anyhow!(err))?;
    codes_with_keymap(combo, &keymap)
}

pub(crate) fn codes_with_keymap(
    combo: &str,
    keymap: &seatgeist_eis::XkbKeymap,
) -> Result<Vec<u16>> {
    match seatgeist_uinput::parse_key_combo(combo) {
        Ok(codes) => return Ok(codes),
        Err(err) => tracing::debug!(%err, "falling back to XKB symbol key-combo lookup"),
    }

    let parts = combo
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        bail!("key combo must contain at least one key");
    }
    if parts.len() > 8 {
        bail!("key combo may contain at most 8 keys");
    }

    parts.iter().map(|part| part_code(part, keymap)).collect()
}

fn part_code(part: &str, keymap: &seatgeist_eis::XkbKeymap) -> Result<u16> {
    if let Ok(codes) = seatgeist_uinput::parse_key_combo(part)
        && let [code] = codes.as_slice()
    {
        return Ok(*code);
    }

    let mut chars = part.chars();
    let Some(character) = chars.next() else {
        bail!("key combo must contain at least one key");
    };
    if chars.next().is_some() {
        bail!("unsupported key name in EIS combo: {part}");
    }

    let keysym =
        seatgeist_eis::unicode_char_to_keysym(character).map_err(|err| anyhow::anyhow!(err))?;
    keymap
        .evdev_keycode_for_keysym_level0(keysym)
        .with_context(|| {
            format!(
                "key combo symbol {character:?} does not map to a level-0 evdev keycode in the current XKB keymap"
            )
        })
}
